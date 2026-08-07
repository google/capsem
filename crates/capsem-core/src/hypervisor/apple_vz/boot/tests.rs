use super::*;
use std::io::Write;

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("capsem-test-boot");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"fake kernel").unwrap();
    path
}

#[test]
fn creates_boot_loader_with_kernel_only() {
    let kernel = temp_file("vmlinuz-boot-test");
    let config = VmConfig::builder().kernel_path(&kernel).build().unwrap();
    let loader = create_boot_loader(&config).unwrap();

    let cmdline = unsafe { loader.commandLine() };
    assert_eq!(
        cmdline.to_string(),
        "console=hvc0 root=/dev/vda ro init_on_alloc=1 slab_nomerge page_alloc.shuffle=1"
    );
    let initrd = unsafe { loader.initialRamdiskURL() };
    assert!(initrd.is_none());
}

#[test]
fn creates_boot_loader_with_custom_cmdline() {
    let kernel = temp_file("vmlinuz-boot-cmd");
    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .kernel_cmdline("console=ttyS0 debug")
        .build()
        .unwrap();
    let loader = create_boot_loader(&config).unwrap();

    let cmdline = unsafe { loader.commandLine() };
    assert_eq!(cmdline.to_string(), "console=ttyS0 debug");
}

#[test]
fn creates_boot_loader_with_initrd() {
    let kernel = temp_file("vmlinuz-boot-initrd");
    let initrd = temp_file("initrd-boot-test.img");
    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .initrd_path(&initrd)
        .build()
        .unwrap();
    let loader = create_boot_loader(&config).unwrap();

    let initrd_url = unsafe { loader.initialRamdiskURL() };
    assert!(initrd_url.is_some());
}

#[test]
fn nsurl_from_valid_path() {
    let url = nsurl_from_path(Path::new("/tmp/test.txt")).unwrap();
    let path = url.path();
    assert!(path.is_some());
    assert_eq!(path.unwrap().to_string(), "/tmp/test.txt");
}
