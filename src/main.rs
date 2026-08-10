fn main() {
    println!(
        "ruut v{} — Copy-on-Write relational database engine",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "Page size: {} bytes | Magic: {:#010X}",
        pager::PAGE_SIZE,
        pager::MAGIC
    );
}
