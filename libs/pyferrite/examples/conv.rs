use pyferrite::prelude::*;
fn main() -> Result<()> {
    let mut a = std::env::args().skip(1);
    let (src, dst) = (a.next().unwrap(), a.next().unwrap());
    convert(&src, &dst, &ReadOptions::new(), &WriteOptions::new())
}
