use std::sync::{Mutex, OnceLock};

pub(crate) const V8_FLAGS: &str = "--no_freeze_flags_after_init --expose-gc";

static V8_LOCK: Mutex<()> = Mutex::new(());
static PLATFORM: OnceLock<v8::SharedRef<v8::Platform>> = OnceLock::new();

pub fn v8_lock() -> std::sync::MutexGuard<'static, ()> {
    V8_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn ensure_v8_initialized() -> &'static v8::SharedRef<v8::Platform> {
    PLATFORM.get_or_init(|| {
        let platform = v8::new_unprotected_default_platform(0, false).make_shared();
        v8::V8::set_flags_from_string(V8_FLAGS);
        v8::V8::initialize_platform(platform.clone());
        v8::cppgc::initialize_process(platform.clone());
        v8::V8::initialize();
        platform
    })
}
