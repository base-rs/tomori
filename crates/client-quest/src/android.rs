// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Android activity and system services, reached through JNI.
use android_activity::AndroidApp;
use anyhow::Result;
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JObject, JString, JValue},
};
use std::{path::PathBuf, sync::Arc};

/// The Java VM and the activity, valid while `android_main` runs.
pub struct Android {
    vm: JavaVM,
    activity: GlobalRef,
    data: Option<PathBuf>,
}

impl Android {
    pub fn new(app: &AndroidApp) -> Result<Arc<Self>> {
        // SAFETY: AndroidApp owns the live VM and Activity references. Create
        // our own GlobalRef so JNI use remains valid for this value's lifetime.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
        let activity = {
            let env = vm.attach_current_thread()?;
            let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
            env.new_global_ref(activity)?
        };
        Ok(Arc::new(Self {
            vm,
            activity,
            data: app.internal_data_path(),
        }))
    }

    /// The app's private storage, which survives restarts and updates.
    pub fn data(&self) -> Option<PathBuf> {
        self.data.clone()
    }

    /// Run `f` with a JNI environment and the activity, inside a local reference frame. A Java exception is cleared and returned as an error.
    pub fn with<T>(
        &self,
        f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>,
    ) -> Result<T> {
        let mut env = self.vm.attach_current_thread()?;
        let result = env.with_local_frame(16, |env| f(env, self.activity.as_obj()));
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        Ok(result?)
    }

    /// A string extra of the intent that started the activity, such as one given with `adb shell am start -e`.
    pub fn intent_extra(&self, name: &str) -> Result<Option<String>> {
        self.with(|env, activity| {
            let intent = env
                .call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?
                .l()?;
            if intent.is_null() {
                return Ok(None);
            }
            let name = env.new_string(name)?;
            let value = env
                .call_method(
                    &intent,
                    "getStringExtra",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            string(env, value)
        })
    }

    /// The device's model name, such as `Quest 3`.
    pub fn device_name(&self) -> Result<Option<String>> {
        self.with(|env, _| {
            let model = env
                .get_static_field("android/os/Build", "MODEL", "Ljava/lang/String;")?
                .l()?;
            string(env, model)
        })
    }

    /// Whether the app holds `permission`, requesting it once if not. The request is answered asynchronously, so check again later.
    pub fn check_or_request(&self, permission: &str, requested: &mut bool) -> Result<bool> {
        self.with(|env, activity| {
            let name = env.new_string(permission)?;
            let granted = env
                .call_method(
                    activity,
                    "checkSelfPermission",
                    "(Ljava/lang/String;)I",
                    &[JValue::Object(&name)],
                )?
                .i()?
                == 0;
            if !granted && !*requested {
                let permissions = env.new_object_array(1, "java/lang/String", &name)?;
                env.call_method(
                    activity,
                    "requestPermissions",
                    "([Ljava/lang/String;I)V",
                    &[JValue::Object(&permissions), JValue::Int(101)],
                )?;
                *requested = true;
            }
            Ok(granted)
        })
    }
}

/// The platform's acoustic echo canceller on one audio session, released when dropped. See `design/spatial-audio.md`.
pub struct EchoCanceller {
    android: Arc<Android>,
    effect: GlobalRef,
}

impl Android {
    /// Log once which of the platform's voice pre-processing effects the device reports, as `AcousticEchoCanceler`, `NoiseSuppressor` and `AutomaticGainControl` say. See `design/client-logging.md`.
    pub fn log_audio_effects(&self) {
        let available = |class: &'static str| {
            self.with(|env, _| {
                env.call_static_method(class, "isAvailable", "()Z", &[])?
                    .z()
            })
        };
        let report = |class| match available(class) {
            Ok(true) => "available",
            Ok(false) => "not available",
            Err(_) => "unknown",
        };
        log::info!(
            "Audio effects: echo canceller {}, noise suppressor {}, automatic gain control {}",
            report("android/media/audiofx/AcousticEchoCanceler"),
            report("android/media/audiofx/NoiseSuppressor"),
            report("android/media/audiofx/AutomaticGainControl")
        );
    }

    /// The platform's echo canceller on audio `session`, enabled if `enable` or else disabled, as the echo test compares; `None` where the device has none or will not create one for it. Logs what it reports.
    pub fn echo_canceller(
        self: &Arc<Self>,
        session: i32,
        enable: bool,
        level: log::Level,
    ) -> Result<Option<EchoCanceller>> {
        const CLASS: &str = "android/media/audiofx/AcousticEchoCanceler";
        let effect = self.with(|env, _| {
            if !env.call_static_method(CLASS, "isAvailable", "()Z", &[])?.z()? {
                log::log!(level, "Echo canceller: not available on this device");
                return Ok(None);
            }
            let effect = env
                .call_static_method(
                    CLASS,
                    "create",
                    "(I)Landroid/media/audiofx/AcousticEchoCanceler;",
                    &[JValue::Int(session)],
                )?
                .l()?;
            if effect.is_null() {
                log::log!(level, "Echo canceller: none created for session {session}");
                return Ok(None);
            }
            let was = env.call_method(&effect, "getEnabled", "()Z", &[])?.z()?;
            let status = if was == enable {
                0
            } else {
                env.call_method(
                    &effect,
                    "setEnabled",
                    "(Z)I",
                    &[JValue::Bool(u8::from(enable))],
                )?
                .i()?
            };
            let enabled = env.call_method(&effect, "getEnabled", "()Z", &[])?.z()?;
            log::log!(
                level,
                "Echo canceller on session {session}: enabled {enabled} (was {was}; asked for {enable}, which returned {status})"
            );
            Ok(Some(env.new_global_ref(effect)?))
        })?;
        Ok(effect.map(|effect| EchoCanceller {
            android: self.clone(),
            effect,
        }))
    }
}

impl Android {
    /// Log, at `level`, what else shapes the live microphone: on its audio `session`, if it has one, whether the platform's noise suppressor and automatic gain control are enabled; and the audio mode, and the call and media volumes, since playing as voice communication uses the call volume. See `design/spatial-audio.md`.
    pub fn log_audio_state(&self, session: Option<i32>, level: log::Level) {
        let effect = |class: &'static str, signature: &'static str| {
            let Some(session) = session else {
                return Ok(None);
            };
            self.with(|env, _| {
                if !env
                    .call_static_method(class, "isAvailable", "()Z", &[])?
                    .z()?
                {
                    return Ok(None);
                }
                let effect = env
                    .call_static_method(class, "create", signature, &[JValue::Int(session)])?
                    .l()?;
                if effect.is_null() {
                    return Ok(None);
                }
                let enabled = env
                    .call_method(&effect, "getEnabled", "()Z", &[])
                    .and_then(|enabled| enabled.z());
                // Released whatever it said, so that only reading it adds nothing to the session; a pending exception is cleared first, as JNI requires.
                if enabled.is_err() {
                    env.exception_clear()?;
                }
                env.call_method(&effect, "release", "()V", &[])?;
                Ok(Some(enabled?))
            })
        };
        let report = |state: Result<Option<bool>>| match state {
            Ok(Some(true)) => "on",
            Ok(Some(false)) => "off",
            Ok(None) => "none",
            Err(_) => "unknown",
        };
        let audio = self.with(|env, activity| {
            let name = env.new_string("audio")?;
            let manager = env
                .call_method(
                    activity,
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let mode = env.call_method(&manager, "getMode", "()I", &[])?.i()?;
            let mut volume = |stream: i32| -> jni::errors::Result<(i32, i32)> {
                Ok((
                    env.call_method(&manager, "getStreamVolume", "(I)I", &[JValue::Int(stream)])?
                        .i()?,
                    env.call_method(
                        &manager,
                        "getStreamMaxVolume",
                        "(I)I",
                        &[JValue::Int(stream)],
                    )?
                    .i()?,
                ))
            };
            // `STREAM_VOICE_CALL` and `STREAM_MUSIC`.
            Ok((mode, volume(0)?, volume(3)?))
        });
        let audio = match audio {
            Ok((mode, (call, call_max), (media, media_max))) => {
                format!(
                    "audio mode {mode}, call volume {call}/{call_max}, media volume {media}/{media_max}"
                )
            }
            Err(error) => format!("audio mode and volumes unknown ({error:#})"),
        };
        let session = session.map_or_else(
            || "without a session".into(),
            |session| format!("session {session}"),
        );
        log::log!(
            level,
            "Live microphone {session}: noise suppressor {}, automatic gain control {}, {audio}",
            report(effect(
                "android/media/audiofx/NoiseSuppressor",
                "(I)Landroid/media/audiofx/NoiseSuppressor;"
            )),
            report(effect(
                "android/media/audiofx/AutomaticGainControl",
                "(I)Landroid/media/audiofx/AutomaticGainControl;"
            )),
        );
    }
}

impl Drop for EchoCanceller {
    fn drop(&mut self) {
        let released = self.android.with(|env, _| {
            env.call_method(self.effect.as_obj(), "release", "()V", &[])
                .map(drop)
        });
        if let Err(error) = released {
            log::debug!("Cannot release the echo canceller: {error:#}");
        }
    }
}

fn string(env: &mut JNIEnv, value: JObject) -> jni::errors::Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(env.get_string(&JString::from(value))?.into()))
}

/// Android drops multicast packets addressed to other hosts unless an app holds a Wi-Fi multicast lock, so Bonjour browsing finds nothing without one. Holding it costs battery, so it is held only while browsing.
pub struct MulticastLock {
    android: Arc<Android>,
    lock: Option<GlobalRef>,
    held: bool,
}

impl MulticastLock {
    pub fn new(android: Arc<Android>) -> Self {
        Self {
            android,
            lock: None,
            held: false,
        }
    }

    pub fn acquire(&mut self) -> Result<()> {
        if self.held {
            return Ok(());
        }
        let lock = match &self.lock {
            Some(lock) => lock.clone(),
            None => {
                let lock = self.create()?;
                self.lock = Some(lock.clone());
                lock
            }
        };
        self.android
            .with(|env, _| env.call_method(lock.as_obj(), "acquire", "()V", &[])?.v())?;
        self.held = true;
        log::info!("Holding the Wi-Fi multicast lock for server discovery");
        Ok(())
    }

    pub fn release(&mut self) {
        if !self.held {
            return;
        }
        self.held = false;
        if let Some(lock) = &self.lock
            && let Err(error) = self
                .android
                .with(|env, _| env.call_method(lock.as_obj(), "release", "()V", &[])?.v())
        {
            log::warn!("Could not release the Wi-Fi multicast lock: {error:#}");
        }
    }

    fn create(&self) -> Result<GlobalRef> {
        self.android.with(|env, activity| {
            let context = env
                .call_method(
                    activity,
                    "getApplicationContext",
                    "()Landroid/content/Context;",
                    &[],
                )?
                .l()?;
            let service = env.new_string("wifi")?;
            let wifi = env
                .call_method(
                    &context,
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[JValue::Object(&service)],
                )?
                .l()?;
            if wifi.is_null() {
                return Err(jni::errors::Error::NullPtr("WifiManager"));
            }
            let tag = env.new_string("tomori-discovery")?;
            let lock = env
                .call_method(
                    &wifi,
                    "createMulticastLock",
                    "(Ljava/lang/String;)Landroid/net/wifi/WifiManager$MulticastLock;",
                    &[JValue::Object(&tag)],
                )?
                .l()?;
            // One acquire and one release, however often browsing restarts.
            env.call_method(&lock, "setReferenceCounted", "(Z)V", &[JValue::Bool(0)])?;
            env.new_global_ref(lock)
        })
    }
}

impl crate::connection::Multicast for MulticastLock {
    fn set_held(&mut self, held: bool) {
        if !held {
            self.release();
        } else if let Err(error) = self.acquire() {
            log::warn!("Could not acquire the Wi-Fi multicast lock: {error:#}");
        }
    }
}

impl Drop for MulticastLock {
    fn drop(&mut self) {
        self.release();
    }
}
