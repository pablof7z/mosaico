use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Vec<(OsString, Option<OsString>)>,
}

impl EnvGuard {
    pub(crate) fn set<K, V>(key: K, value: V) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut guard = Self::lock();
        guard.set_var(key, value);
        guard
    }

    pub(crate) fn remove<K>(key: K) -> Self
    where
        K: AsRef<OsStr>,
    {
        let mut guard = Self::lock();
        guard.remove_var(key);
        guard
    }

    fn lock() -> Self {
        Self {
            _lock: ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
            previous: Vec::new(),
        }
    }

    pub(crate) fn set_var<K, V>(&mut self, key: K, value: V)
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let key = key.as_ref().to_os_string();
        if key == "MOSAICO_HOME" || key == "MOSAICO_CONFIG" {
            self.remove_var("MOSAICO");
        }
        self.remember(&key);
        // SAFETY: lib unit tests that mutate process-global env use this guard,
        // so mutation is serialized and restored before the guard unlocks.
        unsafe { std::env::set_var(key, value) };
    }

    pub(crate) fn remove_var<K>(&mut self, key: K)
    where
        K: AsRef<OsStr>,
    {
        let key = key.as_ref().to_os_string();
        self.remember(&key);
        // SAFETY: lib unit tests that mutate process-global env use this guard,
        // so mutation is serialized and restored before the guard unlocks.
        unsafe { std::env::remove_var(key) };
    }

    fn remember(&mut self, key: &OsStr) {
        if self.previous.iter().any(|(seen, _)| seen == key) {
            return;
        }
        self.previous
            .push((key.to_os_string(), std::env::var_os(key)));
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, previous) in self.previous.iter().rev() {
            match previous {
                Some(value) => {
                    // SAFETY: the guard still holds ENV_LOCK while restoring.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: the guard still holds ENV_LOCK while restoring.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }
    }
}
