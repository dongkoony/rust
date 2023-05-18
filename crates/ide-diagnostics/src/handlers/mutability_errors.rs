use ide_db::source_change::SourceChange;
use syntax::{AstNode, SyntaxKind, SyntaxNode, SyntaxToken, T};
use text_edit::TextEdit;

use crate::{fix, Diagnostic, DiagnosticsContext, Severity};

// Diagnostic: need-mut
//
// This diagnostic is triggered on mutating an immutable variable.
pub(crate) fn need_mut(ctx: &DiagnosticsContext<'_>, d: &hir::NeedMut) -> Diagnostic {
    let fixes = (|| {
        if d.local.is_ref(ctx.sema.db) {
            // There is no simple way to add `mut` to `ref x` and `ref mut x`
            return None;
        }
        let file_id = d.span.file_id.file_id()?;
        let mut edit_builder = TextEdit::builder();
        let use_range = d.span.value.text_range();
        for source in d.local.sources(ctx.sema.db) {
            let Some(ast) = source.name() else { continue };
            edit_builder.insert(ast.syntax().text_range().start(), "mut ".to_string());
        }
        let edit = edit_builder.finish();
        Some(vec![fix(
            "add_mut",
            "Change it to be mutable",
            SourceChange::from_text_edit(file_id, edit),
            use_range,
        )])
    })();
    Diagnostic::new(
        "need-mut",
        format!("cannot mutate immutable variable `{}`", d.local.name(ctx.sema.db)),
        ctx.sema.diagnostics_display_range(d.span.clone()).range,
    )
    .with_fixes(fixes)
}

// Diagnostic: unused-mut
//
// This diagnostic is triggered when a mutable variable isn't actually mutated.
pub(crate) fn unused_mut(ctx: &DiagnosticsContext<'_>, d: &hir::UnusedMut) -> Diagnostic {
    let ast = d.local.primary_source(ctx.sema.db).syntax_ptr();
    let fixes = (|| {
        let file_id = ast.file_id.file_id()?;
        let mut edit_builder = TextEdit::builder();
        let use_range = ast.value.text_range();
        for source in d.local.sources(ctx.sema.db) {
            let ast = source.syntax();
            let Some(mut_token) = token(ast, T![mut]) else { continue };
            edit_builder.delete(mut_token.text_range());
            if let Some(token) = mut_token.next_token() {
                if token.kind() == SyntaxKind::WHITESPACE {
                    edit_builder.delete(token.text_range());
                }
            }
        }
        let edit = edit_builder.finish();
        Some(vec![fix(
            "remove_mut",
            "Remove unnecessary `mut`",
            SourceChange::from_text_edit(file_id, edit),
            use_range,
        )])
    })();
    let ast = d.local.primary_source(ctx.sema.db).syntax_ptr();
    Diagnostic::new(
        "unused-mut",
        "variable does not need to be mutable",
        ctx.sema.diagnostics_display_range(ast).range,
    )
    .severity(Severity::WeakWarning)
    .experimental() // Not supporting `#[allow(unused_mut)]` leads to false positive.
    .with_fixes(fixes)
}

pub(super) fn token(parent: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    parent.children_with_tokens().filter_map(|it| it.into_token()).find(|it| it.kind() == kind)
}

#[cfg(test)]
mod tests {
    use crate::tests::{check_diagnostics, check_fix};

    #[test]
    fn unused_mut_simple() {
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let mut x = 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    f(x);
}
"#,
        );
    }

    #[test]
    fn no_false_positive_simple() {
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let x = 2;
    f(x);
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let mut x = 2;
    x = 5;
    f(x);
}
"#,
        );
    }

    #[test]
    fn multiple_errors_for_single_variable() {
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let x = 2;
    x = 10;
  //^^^^^^ 💡 error: cannot mutate immutable variable `x`
    x = 5;
  //^^^^^ 💡 error: cannot mutate immutable variable `x`
    &mut x;
  //^^^^^^ 💡 error: cannot mutate immutable variable `x`
    f(x);
}
"#,
        );
    }

    #[test]
    fn unused_mut_fix() {
        check_fix(
            r#"
fn f(_: i32) {}
fn main() {
    let mu$0t x = 2;
    f(x);
}
"#,
            r#"
fn f(_: i32) {}
fn main() {
    let x = 2;
    f(x);
}
"#,
        );
        check_fix(
            r#"
fn f(_: i32) {}
fn main() {
    let ((mu$0t x, _) | (_, mut x)) = (2, 3);
    f(x);
}
"#,
            r#"
fn f(_: i32) {}
fn main() {
    let ((x, _) | (_, x)) = (2, 3);
    f(x);
}
"#,
        );
    }

    #[test]
    fn need_mut_fix() {
        check_fix(
            r#"
fn f(_: i32) {}
fn main() {
    let x = 2;
    x$0 = 5;
    f(x);
}
"#,
            r#"
fn f(_: i32) {}
fn main() {
    let mut x = 2;
    x = 5;
    f(x);
}
"#,
        );
        check_fix(
            r#"
fn f(_: i32) {}
fn main() {
    let ((x, _) | (_, x)) = (2, 3);
    x =$0 4;
    f(x);
}
"#,
            r#"
fn f(_: i32) {}
fn main() {
    let ((mut x, _) | (_, mut x)) = (2, 3);
    x = 4;
    f(x);
}
"#,
        );

        check_fix(
            r#"
struct Foo(i32);

impl Foo {
    fn foo(self) {
        self = Fo$0o(5);
    }
}
"#,
            r#"
struct Foo(i32);

impl Foo {
    fn foo(mut self) {
        self = Foo(5);
    }
}
"#,
        );
    }

    #[test]
    fn need_mut_fix_not_applicable_on_ref() {
        check_diagnostics(
            r#"
fn main() {
    let ref x = 2;
    x = &5;
  //^^^^^^ error: cannot mutate immutable variable `x`
}
"#,
        );
        check_diagnostics(
            r#"
fn main() {
    let ref mut x = 2;
    x = &mut 5;
  //^^^^^^^^^^ error: cannot mutate immutable variable `x`
}
"#,
        );
    }

    #[test]
    fn field_mutate() {
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let mut x = (2, 7);
      //^^^^^ 💡 weak: variable does not need to be mutable
    f(x.1);
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let mut x = (2, 7);
    x.0 = 5;
    f(x.1);
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let x = (2, 7);
    x.0 = 5;
  //^^^^^^^ 💡 error: cannot mutate immutable variable `x`
    f(x.1);
}
"#,
        );
    }

    #[test]
    fn mutable_reference() {
        check_diagnostics(
            r#"
fn main() {
    let mut x = &mut 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    *x = 5;
}
"#,
        );
        check_diagnostics(
            r#"
fn main() {
    let x = 2;
    &mut x;
  //^^^^^^ 💡 error: cannot mutate immutable variable `x`
}
"#,
        );
        check_diagnostics(
            r#"
fn main() {
    let x_own = 2;
    let ref mut x_ref = x_own;
      //^^^^^^^^^^^^^ 💡 error: cannot mutate immutable variable `x_own`
}
"#,
        );
        check_diagnostics(
            r#"
struct Foo;
impl Foo {
    fn method(&mut self, x: i32) {}
}
fn main() {
    let x = Foo;
    x.method(2);
  //^ 💡 error: cannot mutate immutable variable `x`
}
"#,
        );
    }

    #[test]
    fn regression_14310() {
        check_diagnostics(
            r#"
            //- minicore: copy, builtin_impls
            fn clone(mut i: &!) -> ! {
                   //^^^^^ 💡 weak: variable does not need to be mutable
                *i
            }
        "#,
        );
    }

    #[test]
    fn match_bindings() {
        check_diagnostics(
            r#"
fn main() {
    match (2, 3) {
        (x, mut y) => {
          //^^^^^ 💡 weak: variable does not need to be mutable
            x = 7;
          //^^^^^ 💡 error: cannot mutate immutable variable `x`
        }
    }
}
"#,
        );
    }

    #[test]
    fn mutation_in_dead_code() {
        // This one is interesting. Dead code is not represented at all in the MIR, so
        // there would be no mutability error for locals in dead code. Rustc tries to
        // not emit `unused_mut` in this case, but since it works without `mut`, and
        // special casing it is not trivial, we emit it.
        check_diagnostics(
            r#"
fn main() {
    return;
    let mut x = 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    &mut x;
}
"#,
        );
        check_diagnostics(
            r#"
fn main() {
    loop {}
    let mut x = 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    &mut x;
}
"#,
        );
        check_diagnostics(
            r#"
enum X {}
fn g() -> X {
    loop {}
}
fn f() -> ! {
    loop {}
}
fn main(b: bool) {
    if b {
        f();
    } else {
        g();
    }
    let mut x = 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    &mut x;
}
"#,
        );
        check_diagnostics(
            r#"
fn main(b: bool) {
    if b {
        loop {}
    } else {
        return;
    }
    let mut x = 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    &mut x;
}
"#,
        );
    }

    #[test]
    fn initialization_is_not_mutation() {
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let mut x;
      //^^^^^ 💡 weak: variable does not need to be mutable
    x = 5;
    f(x);
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main(b: bool) {
    let mut x;
      //^^^^^ 💡 weak: variable does not need to be mutable
    if b {
        x = 1;
    } else {
        x = 3;
    }
    f(x);
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main(b: bool) {
    let x;
    if b {
        x = 1;
    }
    x = 3;
  //^^^^^ 💡 error: cannot mutate immutable variable `x`
    f(x);
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let x;
    loop {
        x = 1;
      //^^^^^ 💡 error: cannot mutate immutable variable `x`
        f(x);
    }
}
"#,
        );
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    loop {
        let mut x = 1;
          //^^^^^ 💡 weak: variable does not need to be mutable
        f(x);
        if let mut y = 2 {
             //^^^^^ 💡 weak: variable does not need to be mutable
            f(y);
        }
        match 3 {
            mut z => f(z),
          //^^^^^ 💡 weak: variable does not need to be mutable
        }
    }
}
"#,
        );
    }

    #[test]
    fn initialization_is_not_mutation_in_loop() {
        check_diagnostics(
            r#"
fn main() {
    let a;
    loop {
        let c @ (
            mut b,
          //^^^^^ 💡 weak: variable does not need to be mutable
            mut d
          //^^^^^ 💡 weak: variable does not need to be mutable
        );
        a = 1;
      //^^^^^ 💡 error: cannot mutate immutable variable `a`
        b = 1;
        c = (2, 3);
        d = 3;
    }
}
"#,
        );
    }

    #[test]
    fn function_arguments_are_initialized() {
        check_diagnostics(
            r#"
fn f(mut x: i32) {
   //^^^^^ 💡 weak: variable does not need to be mutable
}
"#,
        );
        check_diagnostics(
            r#"
fn f(x: i32) {
   x = 5;
 //^^^^^ 💡 error: cannot mutate immutable variable `x`
}
"#,
        );
        check_diagnostics(
            r#"
fn f((x, y): (i32, i32)) {
    let t = [0; 2];
   x = 5;
 //^^^^^ 💡 error: cannot mutate immutable variable `x`
}
"#,
        );
    }

    #[test]
    fn for_loop() {
        check_diagnostics(
            r#"
//- minicore: iterators
fn f(x: [(i32, u8); 10]) {
    for (a, mut b) in x {
          //^^^^^ 💡 weak: variable does not need to be mutable
        a = 2;
      //^^^^^ 💡 error: cannot mutate immutable variable `a`
    }
}
"#,
        );
    }

    #[test]
    fn index() {
        check_diagnostics(
            r#"
//- minicore: coerce_unsized, index, slice
fn f() {
    let x = [1, 2, 3];
    x[2] = 5;
  //^^^^^^^^ 💡 error: cannot mutate immutable variable `x`
    let x = &mut x;
          //^^^^^^ 💡 error: cannot mutate immutable variable `x`
    let mut x = x;
      //^^^^^ 💡 weak: variable does not need to be mutable
    x[2] = 5;
}
"#,
        );
    }

    #[test]
    fn overloaded_index() {
        check_diagnostics(
            r#"
//- minicore: index
use core::ops::{Index, IndexMut};

struct Foo;
impl Index<usize> for Foo {
    type Output = (i32, u8);
    fn index(&self, index: usize) -> &(i32, u8) {
        &(5, 2)
    }
}
impl IndexMut<usize> for Foo {
    fn index_mut(&mut self, index: usize) -> &mut (i32, u8) {
        &mut (5, 2)
    }
}
fn f() {
    let mut x = Foo;
      //^^^^^ 💡 weak: variable does not need to be mutable
    let y = &x[2];
    let x = Foo;
    let y = &mut x[2];
               //^💡 error: cannot mutate immutable variable `x`
    let mut x = &mut Foo;
      //^^^^^ 💡 weak: variable does not need to be mutable
    let y: &mut (i32, u8) = &mut x[2];
    let x = Foo;
    let ref mut y = x[7];
                  //^ 💡 error: cannot mutate immutable variable `x`
    let (ref mut y, _) = x[3];
                       //^ 💡 error: cannot mutate immutable variable `x`
    match x[10] {
        //^ 💡 error: cannot mutate immutable variable `x`
        (ref y, _) => (),
        (_, ref mut y) => (),
    }
    let mut x = Foo;
    let mut i = 5;
      //^^^^^ 💡 weak: variable does not need to be mutable
    let y = &mut x[i];
}
"#,
        );
    }

    #[test]
    fn overloaded_deref() {
        check_diagnostics(
            r#"
//- minicore: deref_mut
use core::ops::{Deref, DerefMut};

struct Foo;
impl Deref for Foo {
    type Target = (i32, u8);
    fn deref(&self) -> &(i32, u8) {
        &(5, 2)
    }
}
impl DerefMut for Foo {
    fn deref_mut(&mut self) -> &mut (i32, u8) {
        &mut (5, 2)
    }
}
fn f() {
    let mut x = Foo;
      //^^^^^ 💡 weak: variable does not need to be mutable
    let y = &*x;
    let x = Foo;
    let y = &mut *x;
               //^^ 💡 error: cannot mutate immutable variable `x`
    let x = Foo;
    let x = Foo;
    let y: &mut (i32, u8) = &mut x;
                          //^^^^^^ 💡 error: cannot mutate immutable variable `x`
    let ref mut y = *x;
                  //^^ 💡 error: cannot mutate immutable variable `x`
    let (ref mut y, _) = *x;
                       //^^ 💡 error: cannot mutate immutable variable `x`
    match *x {
        //^^ 💡 error: cannot mutate immutable variable `x`
        (ref y, _) => (),
        (_, ref mut y) => (),
    }
}
"#,
        );
    }

    #[test]
    fn or_pattern() {
        check_diagnostics(
            r#"
//- minicore: option
fn f(_: i32) {}
fn main() {
    let ((Some(mut x), None) | (_, Some(mut x))) = (None, Some(7));
             //^^^^^ 💡 weak: variable does not need to be mutable
    f(x);
}
"#,
        );
    }

    #[test]
    fn or_pattern_no_terminator() {
        check_diagnostics(
            r#"
enum Foo {
    A, B, C, D
}

use Foo::*;

fn f(inp: (Foo, Foo, Foo, Foo)) {
    let ((A, B, _, x) | (B, C | D, x, _)) = inp else {
        return;
    };
    x = B;
  //^^^^^ 💡 error: cannot mutate immutable variable `x`
}
"#,
        );
    }

    #[test]
    // FIXME: We should have tests for `is_ty_uninhabited_from`
    fn regression_14421() {
        check_diagnostics(
            r#"
pub enum Tree {
    Node(TreeNode),
    Leaf(TreeLeaf),
}

struct Box<T>(&T);

pub struct TreeNode {
    pub depth: usize,
    pub children: [Box<Tree>; 8]
}

pub struct TreeLeaf {
    pub depth: usize,
    pub data: u8
}

pub fn test() {
    let mut tree = Tree::Leaf(
      //^^^^^^^^ 💡 weak: variable does not need to be mutable
        TreeLeaf {
            depth: 0,
            data: 0
        }
    );
}
"#,
        );
    }

    #[test]
    fn fn_traits() {
        check_diagnostics(
            r#"
//- minicore: fn
fn fn_ref(mut x: impl Fn(u8) -> u8) -> u8 {
        //^^^^^ 💡 weak: variable does not need to be mutable
    x(2)
}
fn fn_mut(x: impl FnMut(u8) -> u8) -> u8 {
    x(2)
  //^ 💡 error: cannot mutate immutable variable `x`
}
fn fn_borrow_mut(mut x: &mut impl FnMut(u8) -> u8) -> u8 {
               //^^^^^ 💡 weak: variable does not need to be mutable
    x(2)
}
fn fn_once(mut x: impl FnOnce(u8) -> u8) -> u8 {
         //^^^^^ 💡 weak: variable does not need to be mutable
    x(2)
}
"#,
        );
    }

    #[test]
    fn closure() {
        // FIXME: Diagnostic spans are inconsistent inside and outside closure
        check_diagnostics(
            r#"
        //- minicore: copy, fn
        struct X;

        impl X {
            fn mutate(&mut self) {}
        }

        fn f() {
            let x = 5;
            let closure1 = || { x = 2; };
                              //^ 💡 error: cannot mutate immutable variable `x`
            let _ = closure1();
                  //^^^^^^^^ 💡 error: cannot mutate immutable variable `closure1`
            let closure2 = || { x = x; };
                              //^ 💡 error: cannot mutate immutable variable `x`
            let closure3 = || {
                let x = 2;
                x = 5;
              //^^^^^ 💡 error: cannot mutate immutable variable `x`
                x
            };
            let x = X;
            let closure4 = || { x.mutate(); };
                              //^ 💡 error: cannot mutate immutable variable `x`
        }
                    "#,
        );
        check_diagnostics(
            r#"
        //- minicore: copy, fn
        fn f() {
            let mut x = 5;
              //^^^^^ 💡 weak: variable does not need to be mutable
            let mut y = 2;
            y = 7;
            let closure = || {
                let mut z = 8;
                z = 3;
                let mut k = z;
                  //^^^^^ 💡 weak: variable does not need to be mutable
            };
        }
                    "#,
        );
        check_diagnostics(
            r#"
//- minicore: copy, fn
fn f() {
    let closure = || {
        || {
            || {
                let x = 2;
                || { || { x = 5; } }
                        //^ 💡 error: cannot mutate immutable variable `x`
            }
        }
    };
}
            "#,
        );
        check_diagnostics(
            r#"
//- minicore: copy, fn
fn f() {
    struct X;
    let mut x = X;
      //^^^^^ 💡 weak: variable does not need to be mutable
    let c1 = || x;
    let mut x = X;
    let c2 = || { x = X; x };
    let mut x = X;
    let c2 = move || { x = X; };
}
            "#,
        );
        check_diagnostics(
            r#"
        //- minicore: copy, fn, deref_mut
        struct X(i32, i64);

        fn f() {
            let mut x = &mut 5;
              //^^^^^ 💡 weak: variable does not need to be mutable
            let closure1 = || { *x = 2; };
            let _ = closure1();
                  //^^^^^^^^ 💡 error: cannot mutate immutable variable `closure1`
            let mut x = &mut 5;
              //^^^^^ 💡 weak: variable does not need to be mutable
            let closure1 = || { *x = 2; &x; };
            let _ = closure1();
                  //^^^^^^^^ 💡 error: cannot mutate immutable variable `closure1`
            let mut x = &mut 5;
            let closure1 = || { *x = 2; &x; x = &mut 3; };
            let _ = closure1();
                  //^^^^^^^^ 💡 error: cannot mutate immutable variable `closure1`
            let mut x = &mut 5;
              //^^^^^ 💡 weak: variable does not need to be mutable
            let closure1 = move || { *x = 2; };
            let _ = closure1();
                  //^^^^^^^^ 💡 error: cannot mutate immutable variable `closure1`
            let mut x = &mut X(1, 2);
              //^^^^^ 💡 weak: variable does not need to be mutable
            let closure1 = || { x.0 = 2; };
            let _ = closure1();
                  //^^^^^^^^ 💡 error: cannot mutate immutable variable `closure1`
        }
                    "#,
        );
    }

    #[test]
    fn allow_unused_mut_for_identifiers_starting_with_underline() {
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    let mut _x = 2;
    f(_x);
}
"#,
        );
    }

    #[test]
    fn respect_allow_unused_mut() {
        // FIXME: respect
        check_diagnostics(
            r#"
fn f(_: i32) {}
fn main() {
    #[allow(unused_mut)]
    let mut x = 2;
      //^^^^^ 💡 weak: variable does not need to be mutable
    f(x);
}
"#,
        );
    }
}
