#[must_use]
pub fn runtime_id_hex(runtime_id: &[i32]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(runtime_id.len().saturating_mul(8));
    for part in runtime_id {
        if write!(&mut output, "{:08x}", part.cast_unsigned()).is_err() {
            break;
        }
    }
    output
}
