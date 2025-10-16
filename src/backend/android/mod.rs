use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::errors::*;
use crate::Ignore;

use jni::errors::Error as JniError;
use jni::objects::{GlobalRef, JObject, JObjectArray, JString, JValue};
use jni::sys::{jint, jobject, JNIEnv as JNISys};
use jni::JNIEnv;

use jni_min_helper::{android_context, jni_with_env, JObjectGet, JniProxy};

// AMidi (NDK) FFI

#[allow(non_camel_case_types)]
type media_status_t = i32;

// Opaque types
#[repr(C)]
struct AMidiDevice;
#[repr(C)]
struct AMidiInputPort;
#[repr(C)]
struct AMidiOutputPort;

// opcodes for AMidiOutputPort_receive
const AMIDI_OPCODE_DATA: i32 = 1;
const AMIDI_OPCODE_FLUSH: i32 = 2;

#[cfg(target_os = "android")]
#[link(name = "amidi")]
extern "C" {
    fn AMidiDevice_fromJava(
        env: *mut JNISys,
        midiDeviceObj: jobject,
        outDevicePtrPtr: *mut *mut AMidiDevice,
    ) -> media_status_t;
    fn AMidiDevice_release(device: *mut AMidiDevice) -> media_status_t;

    fn AMidiInputPort_open(
        device: *const AMidiDevice,
        port_number: i32,
        out_input_port: *mut *mut AMidiInputPort,
    ) -> media_status_t;
    fn AMidiInputPort_close(input_port: *mut AMidiInputPort);

    fn AMidiInputPort_send(
        input_port: *mut AMidiInputPort,
        buffer: *const u8,
        num_bytes: usize,
    ) -> isize;

    fn AMidiOutputPort_open(
        device: *const AMidiDevice,
        port_number: i32,
        out_output_port: *mut *mut AMidiOutputPort,
    ) -> media_status_t;
    fn AMidiOutputPort_close(output_port: *mut AMidiOutputPort);

    fn AMidiOutputPort_receive(
        output_port: *mut AMidiOutputPort,
        opcode_ptr: *mut i32,
        buffer: *mut u8,
        max_bytes: usize,
        num_bytes_received_ptr: *mut usize,
        out_timestamp_ns_ptr: *mut i64,
    ) -> isize;
}

// Helpers (JNI)

fn get_midi_manager<'a>(env: &mut JNIEnv<'a>) -> Result<JObject<'a>, InitError> {
    // Acquire a local ref for the app Context
    let ctx_local = env
        .new_local_ref(android_context())
        .map_err(|_| InitError)?;

    let class_ctx = env
        .find_class("android/content/Context")
        .map_err(|_| InitError)?;
    let midi_service_field = env
        .get_static_field(&class_ctx, "MIDI_SERVICE", "Ljava/lang/String;")
        .map_err(|_| InitError)?
        .l()
        .map_err(|_| InitError)?;

    let mgr = env
        .call_method(
            &ctx_local,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::from(&midi_service_field)],
        )
        .map_err(|_| InitError)?
        .l()
        .map_err(|_| InitError)?;

    Ok(mgr)
}

fn java_string(env: &mut JNIEnv<'_>, s: JString<'_>) -> String {
    env.get_string(&s)
        .map(|os| os.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn get_devices<'a>(
    env: &mut JNIEnv<'a>,
    midi_manager: &JObject<'a>,
) -> Result<Vec<JObject<'a>>, InitError> {
    let devices_obj = env
        .call_method(
            midi_manager,
            "getDevices",
            "()[Landroid/media/midi/MidiDeviceInfo;",
            &[],
        )
        .map_err(|_| InitError)?
        .l()
        .map_err(|_| InitError)?;

    let arr: JObjectArray<'_> = devices_obj.into();
    let len = env.get_array_length(&arr).map_err(|_| InitError)? as i32;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let obj = env
            .get_object_array_element(&arr, i)
            .map_err(|_| InitError)?;
        out.push(obj);
    }
    Ok(out)
}

fn port_label<'a>(env: &mut JNIEnv<'a>, info: &JObject<'a>, port_info: &JObject<'a>) -> String {
    let dev = (|| -> Result<String, JniError> {
        let info_cls = env.find_class("android/media/midi/MidiDeviceInfo")?;
        let props = env
            .call_method(info, "getProperties", "()Landroid/os/Bundle;", &[])?
            .l()?;
        let key = env
            .get_static_field(&info_cls, "PROPERTY_NAME", "Ljava/lang/String;")?
            .l()?;
        let name_obj = env
            .call_method(
                &props,
                "getString",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::from(&key)],
            )?
            .l()?;
        let name: JString<'_> = JString::from(name_obj);
        Ok(java_string(env, name))
    })()
    .unwrap_or_else(|_| "MIDI Device".to_owned());

    let port_name = (|| -> Result<String, JniError> {
        let name_obj = env
            .call_method(port_info, "getName", "()Ljava/lang/String;", &[])?
            .l()?;
        let s: JString<'_> = JString::from(name_obj);
        Ok(java_string(env, s))
    })()
    .unwrap_or_else(|_| "Port".to_owned());

    let port_number = env
        .call_method(port_info, "getPortNumber", "()I", &[])
        .and_then(|v| v.i())
        .unwrap_or(0);

    format!("{dev} – {port_name} (#{port_number})")
}

fn open_midi_device_global<'a>(
    env: &mut JNIEnv<'a>,
    info: &JObject<'a>,
    midi_manager: &JObject<'a>,
) -> Result<GlobalRef, InitError> {
    // Prepare a oneshot channel to receive the device object.
    let (tx, rx) = mpsc::channel::<Option<GlobalRef>>();

    // Build a dynamic proxy for MidiManager.OnDeviceOpenedListener
    let listener = JniProxy::build(
        env,
        None,
        ["android/media/midi/MidiManager$OnDeviceOpenedListener"].as_slice(),
        move |env, method, args| {
            if method.get_method_name(env).unwrap_or_default() == "onDeviceOpened" {
                // args[0] is MidiDevice or null
                let dev = if args[0].is_null() {
                    None
                } else {
                    env.new_global_ref(&args[0]).ok()
                };
                let _ = tx.send(dev);
            }
            JniProxy::void(env)
        },
    )
    .map_err(|_| InitError)?;

    // Call openDevice(info, listener, null)
    let null_obj = JObject::null();
    env.call_method(
        midi_manager,
        "openDevice",
        "(Landroid/media/midi/MidiDeviceInfo;Landroid/media/midi/MidiManager$OnDeviceOpenedListener;Landroid/os/Handler;)V",
        &[JValue::from(info), JValue::from(&listener), JValue::from(&null_obj)],
    )
    .map_err(|_| InitError)?;

    // Wait up to 5 seconds
    let dev = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| InitError)?;
    dev.ok_or(InitError)
}

unsafe fn amidi_from_java(
    env: &mut JNIEnv<'_>,
    device_obj: &GlobalRef,
) -> Result<*mut AMidiDevice, InitError> {
    // To pass to the NDK function
    let local = env
        .new_local_ref(device_obj.as_obj())
        .map_err(|_| InitError)?;
    let mut out: *mut AMidiDevice = std::ptr::null_mut();
    let status = AMidiDevice_fromJava(
        env.get_native_interface(),
        local.into_raw(),
        &mut out as *mut _,
    );
    if status != 0 || out.is_null() {
        return Err(InitError);
    }
    Ok(out)
}

fn close_java_device(env: &mut JNIEnv<'_>, device_obj: &GlobalRef) {
    let _ = env.call_method(device_obj.as_obj(), "close", "()V", &[]);
}

// Public types

#[derive(Clone, PartialEq)]
pub struct MidiInputPort {
    device_id: i32,
    port_number: i32,
    name: String,
}

impl MidiInputPort {
    pub fn id(&self) -> String {
        format!("{}:in:{}", self.device_id, self.port_number)
    }
}

pub struct MidiInput {
    ignore_flags: Ignore,
}

impl MidiInput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        // To ensure JNI is available and an Android context exists
        let _ = jni_with_env(|_env| Ok(())).map_err(|_| InitError)?;
        Ok(MidiInput {
            ignore_flags: Ignore::None,
        })
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiInputPort> {
        jni_with_env(|env| -> Result<_, JniError> {
            let mgr = get_midi_manager(env).map_err(|_| JniError::NullPtr("get_midi_manager"))?;
            let devices = get_devices(env, &mgr).map_err(|_| JniError::NullPtr("get_devices"))?;
            let pi_class = env.find_class("android/media/midi/MidiDeviceInfo$PortInfo")?;
            let type_output = env.get_static_field(&pi_class, "TYPE_OUTPUT", "I")?.i()?;

            let mut result = Vec::new();
            for info in devices {
                let id = env.call_method(&info, "getId", "()I", &[])?.i()?;
                let ports = env
                    .call_method(
                        &info,
                        "getPorts",
                        "()[Landroid/media/midi/MidiDeviceInfo$PortInfo;",
                        &[],
                    )?
                    .l()?;
                let arr: JObjectArray<'_> = ports.into();
                let len = env.get_array_length(&arr)? as i32;

                for i in 0..len {
                    let port_info = env.get_object_array_element(&arr, i)?;
                    let ptype = env.call_method(&port_info, "getType", "()I", &[])?.i()?;
                    if ptype == type_output {
                        let pnum = env
                            .call_method(&port_info, "getPortNumber", "()I", &[])?
                            .i()?;
                        let label = port_label(env, &info, &port_info);
                        result.push(crate::common::MidiInputPort {
                            imp: MidiInputPort {
                                device_id: id,
                                port_number: pnum,
                                name: label,
                            },
                        });
                    }
                }
            }
            Ok(result)
        })
        .unwrap_or_default()
    }

    pub fn port_count(&self) -> usize {
        self.ports_internal().len()
    }

    pub fn port_name(&self, port: &MidiInputPort) -> Result<String, PortInfoError> {
        Ok(port.name.clone())
    }

    pub fn connect<F, T: Send + 'static>(
        self,
        port: &MidiInputPort,
        _port_name: &str,
        mut callback: F,
        data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        let ignore_flags = self.ignore_flags;

        let open = jni_with_env(|env| -> Result<_, JniError> {
            let mgr = get_midi_manager(env).map_err(|_| JniError::NullPtr("get_midi_manager"))?;
            let devices = get_devices(env, &mgr).map_err(|_| JniError::NullPtr("get_devices"))?;
            let info = devices
                .into_iter()
                .find(|info| {
                    env.call_method(info, "getId", "()I", &[])
                        .and_then(|v| v.i())
                        .map(|id| id == port.device_id)
                        .unwrap_or(false)
                })
                .ok_or(JniError::NullPtr("device not found"))?;

            let dev_global = open_midi_device_global(env, &info, &mgr)
                .map_err(|_| JniError::NullPtr("open_device"))?;
            let amidi = unsafe { amidi_from_java(env, &dev_global) }
                .map_err(|_| JniError::NullPtr("amidi_from_java"))?;
            Ok((dev_global, amidi))
        });

        let (java_device, amidi_device) = match open {
            Ok(v) => v,
            Err(_) => return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self)),
        };

        // Open device output port for reading
        let mut out_port: *mut AMidiOutputPort = std::ptr::null_mut();
        let status = unsafe {
            AMidiOutputPort_open(
                amidi_device as *const AMidiDevice,
                port.port_number,
                &mut out_port,
            )
        };
        if status != 0 || out_port.is_null() {
            unsafe { AMidiDevice_release(amidi_device) };
            let _ = jni_with_env(|env| {
                close_java_device(env, &java_device);
                Ok(())
            });
            return Err(ConnectError::other(
                "could not open Android MIDI output port",
                self,
            ));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let out_port_addr = out_port as usize;

        // Spawn reader thread
        let thread: JoinHandle<T> = thread::Builder::new()
            .name("midir Android input handler".to_string())
            .spawn(move || {
                let out_port_ptr = out_port_addr as *mut AMidiOutputPort;
                let mut buf = vec![0u8; 1024];
                let mut opcode: i32 = 0;
                let mut nbytes: usize = 0;
                let mut ts_ns: i64 = 0;

                let mut user_data = data;

                while !stop_clone.load(Ordering::Relaxed) {
                    let rc = unsafe {
                        AMidiOutputPort_receive(
                            out_port_ptr,
                            &mut opcode as *mut _,
                            buf.as_mut_ptr(),
                            buf.len(),
                            &mut nbytes as *mut _,
                            &mut ts_ns as *mut _,
                        )
                    };
                    if rc < 0 {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }

                    if opcode == AMIDI_OPCODE_FLUSH {
                        continue;
                    }

                    if opcode == AMIDI_OPCODE_DATA && nbytes > 0 {
                        let message = &buf[..nbytes];
                        let status = message[0];

                        // Filter according to Ignore flags
                        if (status == 0xF0 && ignore_flags.contains(Ignore::Sysex))
                            || (status == 0xF1 && ignore_flags.contains(Ignore::Time))
                            || (status == 0xF8 && ignore_flags.contains(Ignore::Time))
                            || (status == 0xFE && ignore_flags.contains(Ignore::ActiveSense))
                        {
                            continue;
                        }

                        let ts_us = if ts_ns > 0 { (ts_ns as u64) / 1000 } else { 0 };
                        callback(ts_us, message, &mut user_data);
                    } else {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }

                user_data
            })
            .map_err(|_| {
                ConnectError::other("could not start Android input handler thread", self)
            })?;

        Ok(MidiInputConnection {
            java_device,
            amidi_device,
            out_port,
            stop,
            thread: Some(thread),
            ignore_flags,
        })
    }

    // Virtual ports are not supported when using AMidi without a MidiDeviceService (Just throw a err)
    pub fn create_virtual<F, T: Send>(
        self,
        _port_name: &str,
        _callback: F,
        _data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        Err(ConnectError::other(
            "virtual MIDI input ports are not supported on Android",
            self,
        ))
    }
}

unsafe impl<T: Send> Send for MidiInputConnection<T> {}
unsafe impl<T: Send> Sync for MidiInputConnection<T> {}

pub struct MidiInputConnection<T> {
    java_device: GlobalRef,
    amidi_device: *mut AMidiDevice,
    out_port: *mut AMidiOutputPort,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<T>>,
    ignore_flags: Ignore,
}

impl<T> MidiInputConnection<T> {
    pub fn close(mut self) -> (MidiInput, T) {
        self.stop.store(true, Ordering::Relaxed);
        let user_data = self.thread.take().map(|h| h.join().unwrap()).unwrap();

        unsafe {
            AMidiOutputPort_close(self.out_port);
            AMidiDevice_release(self.amidi_device);
        }
        let _ = jni_with_env(|env| {
            close_java_device(env, &self.java_device);
            Ok(())
        });

        (
            MidiInput {
                ignore_flags: self.ignore_flags,
            },
            user_data,
        )
    }
}

#[derive(Clone, PartialEq)]
pub struct MidiOutputPort {
    device_id: i32,
    port_number: i32,
    name: String,
}

impl MidiOutputPort {
    pub fn id(&self) -> String {
        format!("{}:out:{}", self.device_id, self.port_number)
    }
}

pub struct MidiOutput {
    // no state
}

impl MidiOutput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        let _ = jni_with_env(|_env| Ok(())).map_err(|_| InitError)?;
        Ok(MidiOutput {})
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiOutputPort> {
        jni_with_env(|env| -> Result<_, JniError> {
            let mgr = get_midi_manager(env).map_err(|_| JniError::NullPtr("get_midi_manager"))?;
            let devices = get_devices(env, &mgr).map_err(|_| JniError::NullPtr("get_devices"))?;
            let pi_class = env.find_class("android/media/midi/MidiDeviceInfo$PortInfo")?;
            let type_input = env.get_static_field(&pi_class, "TYPE_INPUT", "I")?.i()?;

            let mut result = Vec::new();
            for info in devices {
                let id = env.call_method(&info, "getId", "()I", &[])?.i()?;
                let ports = env
                    .call_method(
                        &info,
                        "getPorts",
                        "()[Landroid/media/midi/MidiDeviceInfo$PortInfo;",
                        &[],
                    )?
                    .l()?;
                let arr: JObjectArray<'_> = ports.into();
                let len = env.get_array_length(&arr)? as i32;

                for i in 0..len {
                    let port_info = env.get_object_array_element(&arr, i)?;
                    let ptype = env.call_method(&port_info, "getType", "()I", &[])?.i()?;
                    if ptype == type_input {
                        let pnum = env
                            .call_method(&port_info, "getPortNumber", "()I", &[])?
                            .i()?;
                        let label = port_label(env, &info, &port_info);
                        result.push(crate::common::MidiOutputPort {
                            imp: MidiOutputPort {
                                device_id: id,
                                port_number: pnum,
                                name: label,
                            },
                        });
                    }
                }
            }
            Ok(result)
        })
        .unwrap_or_default()
    }

    pub fn port_count(&self) -> usize {
        self.ports_internal().len()
    }

    pub fn port_name(&self, port: &MidiOutputPort) -> Result<String, PortInfoError> {
        Ok(port.name.clone())
    }

    pub fn connect(
        self,
        port: &MidiOutputPort,
        _port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let open = jni_with_env(|env| -> Result<_, JniError> {
            let mgr = get_midi_manager(env).map_err(|_| JniError::NullPtr("get_midi_manager"))?;
            let devices = get_devices(env, &mgr).map_err(|_| JniError::NullPtr("get_devices"))?;
            let info = devices
                .into_iter()
                .find(|info| {
                    env.call_method(info, "getId", "()I", &[])
                        .and_then(|v| v.i())
                        .map(|id| id == port.device_id)
                        .unwrap_or(false)
                })
                .ok_or(JniError::NullPtr("device not found"))?;

            let dev_global = open_midi_device_global(env, &info, &mgr)
                .map_err(|_| JniError::NullPtr("open_device"))?;
            let amidi = unsafe { amidi_from_java(env, &dev_global) }
                .map_err(|_| JniError::NullPtr("amidi_from_java"))?;
            Ok((dev_global, amidi))
        });

        let (java_device, amidi_device) = match open {
            Ok(v) => v,
            Err(_) => return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self)),
        };

        // Open device input port for sending
        let mut in_port: *mut AMidiInputPort = std::ptr::null_mut();
        let status = unsafe {
            AMidiInputPort_open(
                amidi_device as *const AMidiDevice,
                port.port_number,
                &mut in_port,
            )
        };
        if status != 0 || in_port.is_null() {
            unsafe { AMidiDevice_release(amidi_device) };
            let _ = jni_with_env(|env| {
                close_java_device(env, &java_device);
                Ok(())
            });
            return Err(ConnectError::other(
                "could not open Android MIDI input port",
                self,
            ));
        }

        Ok(MidiOutputConnection {
            java_device,
            amidi_device,
            in_port,
        })
    }

    // Similar
    pub fn create_virtual(
        self,
        _port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<Self>> {
        Err(ConnectError::other(
            "virtual MIDI output ports are not supported on Android",
            self,
        ))
    }
}

unsafe impl Sync for MidiOutputConnection {}

pub struct MidiOutputConnection {
    java_device: GlobalRef,
    amidi_device: *mut AMidiDevice,
    in_port: *mut AMidiInputPort,
}

unsafe impl Send for MidiOutputConnection {}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        unsafe {
            AMidiInputPort_close(self.in_port);
            AMidiDevice_release(self.amidi_device);
        }
        let _ = jni_with_env(|env| {
            close_java_device(env, &self.java_device);
            Ok(())
        });
        MidiOutput {}
    }

    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        if message.is_empty() {
            return Err(SendError::InvalidData(
                "message to be sent must not be empty",
            ));
        }

        let rc = unsafe { AMidiInputPort_send(self.in_port, message.as_ptr(), message.len()) };
        if rc < 0 {
            return Err(SendError::Other("AMidiInputPort_send failed"));
        }

        Ok(())
    }
}
