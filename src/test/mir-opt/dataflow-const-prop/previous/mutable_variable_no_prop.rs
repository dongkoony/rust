// unit-test: DataflowConstProp

static mut STATIC: u32 = 42;

// EMIT_MIR mutable_variable_no_prop.main.DataflowConstProp.diff
fn main() {
    let mut x = 42;
    unsafe {
        x = STATIC;
    }
    let y = x;
}
