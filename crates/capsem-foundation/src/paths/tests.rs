use super::*;

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    crate::TEST_ENV_LOCK.lock().unwrap()
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn capsem_home_uses_env_var_when_set() {
    let _lock = lock_env();
    let _g = EnvGuard::set("CAPSEM_HOME", "/tmp/test-capsem-home");
    assert_eq!(capsem_home(), PathBuf::from("/tmp/test-capsem-home"));
}

#[test]
fn capsem_home_ignores_empty_env_var() {
    let _lock = lock_env();
    let _g = EnvGuard::set("CAPSEM_HOME", "");
    let _h = EnvGuard::set("HOME", "/home/alice");
    assert_eq!(capsem_home(), PathBuf::from("/home/alice/.capsem"));
}

#[test]
fn capsem_home_falls_back_to_home() {
    let _lock = lock_env();
    let _g = EnvGuard::unset("CAPSEM_HOME");
    let _h = EnvGuard::set("HOME", "/home/bob");
    assert_eq!(capsem_home(), PathBuf::from("/home/bob/.capsem"));
}

#[test]
fn run_dir_honors_env_override_over_home() {
    let _lock = lock_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/isolated");
    let _r = EnvGuard::set("CAPSEM_RUN_DIR", "/tmp/custom-run");
    assert_eq!(capsem_run_dir(), PathBuf::from("/tmp/custom-run"));
}

#[test]
fn run_dir_under_isolated_home() {
    let _lock = lock_env();
    let _r = EnvGuard::unset("CAPSEM_RUN_DIR");
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/isolated");
    assert_eq!(capsem_run_dir(), PathBuf::from("/tmp/isolated/run"));
}

#[test]
fn assets_dir_honors_env_override_over_home() {
    let _lock = lock_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/isolated");
    let _a = EnvGuard::set("CAPSEM_ASSETS_DIR", "/repo/assets");
    assert_eq!(capsem_assets_dir(), PathBuf::from("/repo/assets"));
}

#[test]
fn assets_dir_under_isolated_home() {
    let _lock = lock_env();
    let _a = EnvGuard::unset("CAPSEM_ASSETS_DIR");
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/isolated");
    assert_eq!(capsem_assets_dir(), PathBuf::from("/tmp/isolated/assets"));
}

#[test]
fn sessions_dir_under_isolated_home() {
    let _lock = lock_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/isolated");
    assert_eq!(capsem_sessions_dir(), PathBuf::from("/tmp/isolated/sessions"));
}

#[test]
fn service_socket_and_pidfile_under_run_dir() {
    let _lock = lock_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/isolated");
    let _r = EnvGuard::unset("CAPSEM_RUN_DIR");
    assert_eq!(service_socket_path(), PathBuf::from("/tmp/isolated/run/service.sock"));
    assert_eq!(service_pidfile_path(), PathBuf::from("/tmp/isolated/run/service.pid"));
}
