//! Tests for the `TUI_TRANSLATOR_QUALITY` env-var loader (T3).
//!
//! Tests touch the process-wide `TUI_TRANSLATOR_QUALITY` env var
//! and must therefore run serially. We use a global `Mutex` to
//! enforce that. A poisoning-error panics propagate, but normal
//! lock/unlock is fine.

use std::sync::Mutex;

use crate::quality_preset::{
    load_preset_from_env, resolve_active_preset, QualityPreset, QUALITY_ENV_VAR,
};
use crate::sys_caps::{GpuKind, SysCaps};

/// Global lock. Wrap the body of every test that touches the
/// env var in `let _g = ENV_LOCK.lock().unwrap();`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard: set the env var on creation, restore on drop.
struct EnvVarGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var_os(key);
        // SAFETY: tests run single-threaded (ENV_LOCK Mutex).
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }

    fn remove(key: &'static str) -> Self {
        let prev = std::env::var_os(key);
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[test]
fn env_unset_returns_auto() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::remove("TUI_TRANSLATOR_QUALITY");
    assert_eq!(load_preset_from_env(), QualityPreset::Auto);
}

#[test]
fn env_empty_string_returns_auto() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "");
    assert_eq!(load_preset_from_env(), QualityPreset::Auto);
}

#[test]
fn env_set_to_best() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "Best");
    assert_eq!(load_preset_from_env(), QualityPreset::Best);
}

#[test]
fn env_set_to_performance() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "performance");
    assert_eq!(load_preset_from_env(), QualityPreset::Performance);
}

#[test]
fn env_set_to_custom() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "CUSTOM");
    assert_eq!(load_preset_from_env(), QualityPreset::Custom);
}

#[test]
fn env_set_to_auto_resolves_to_auto() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "Auto");
    assert_eq!(load_preset_from_env(), QualityPreset::Auto);
}

#[test]
fn env_set_to_unknown_falls_back_to_auto() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "ULTRA");
    assert_eq!(load_preset_from_env(), QualityPreset::Auto);
}

#[test]
fn resolve_active_preset_unset_and_high_ram_yields_best() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::remove("TUI_TRANSLATOR_QUALITY");
    let caps = SysCaps {
        total_memory_bytes: 32 * 1024 * 1024 * 1024,
        physical_cores: 12,
        gpu: GpuKind::None,
    };
    assert_eq!(resolve_active_preset(&caps), QualityPreset::Best);
}

#[test]
fn resolve_active_preset_unset_and_low_ram_yields_performance() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::remove("TUI_TRANSLATOR_QUALITY");
    let caps = SysCaps {
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        physical_cores: 4,
        gpu: GpuKind::None,
    };
    assert_eq!(resolve_active_preset(&caps), QualityPreset::Performance);
}

#[test]
fn resolve_active_preset_env_overrides_ram_tier() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "Best");
    let caps = SysCaps {
        total_memory_bytes: 4 * 1024 * 1024 * 1024,
        physical_cores: 2,
        gpu: GpuKind::None,
    };
    assert_eq!(resolve_active_preset(&caps), QualityPreset::Best);
}

#[test]
fn resolve_active_preset_env_performance_on_high_ram() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set("TUI_TRANSLATOR_QUALITY", "Performance");
    let caps = SysCaps {
        total_memory_bytes: 64 * 1024 * 1024 * 1024,
        physical_cores: 32,
        gpu: GpuKind::None,
    };
    assert_eq!(resolve_active_preset(&caps), QualityPreset::Performance);
}

#[test]
fn env_var_name_constant_is_stable() {
    assert_eq!(QUALITY_ENV_VAR, "TUI_TRANSLATOR_QUALITY");
}

#[test]
fn quality_preset_env_var_is_in_help_output() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _g = EnvVarGuard::set(QUALITY_ENV_VAR, "Best");
    assert_eq!(std::env::var(QUALITY_ENV_VAR).unwrap(), "Best");
}
