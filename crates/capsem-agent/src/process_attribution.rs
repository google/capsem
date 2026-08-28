const MAX_PROCESS_NAME_CHARS: usize = 128;
const META_PREFIX: &[u8] = b"\0CAPSEM_META:";

pub fn sanitize_process_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() || c == ' ' { '_' } else { c })
        .take(MAX_PROCESS_NAME_CHARS)
        .collect()
}

pub fn encode_meta_line(name: &str) -> Vec<u8> {
    let name = sanitize_process_name(name);
    let mut meta = Vec::with_capacity(META_PREFIX.len() + name.len() + 1);
    meta.extend_from_slice(META_PREFIX);
    meta.extend_from_slice(name.as_bytes());
    meta.push(b'\n');
    meta
}

#[cfg(test)]
#[path = "process_attribution/tests.rs"]
mod tests;
