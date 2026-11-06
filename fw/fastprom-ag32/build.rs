//! Writes memory.x (the application's window behind its header) and
//! build_info.rs, and links with riscv-rt's link.x: fw/fastprom-hal/build_support.rs.

include!("../fastprom-hal/build_support.rs");

fn main() {
    build(Binary::Application);
}
