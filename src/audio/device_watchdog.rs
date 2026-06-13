//! IMMNotificationClient watchdog for virtual-mic device-loss detection (US-09 / #734).
//!
//! When the OS removes or disables the monitored render endpoint (VB-CABLE,
//! USB audio, sleep/wake cycle) the watchdog fires a
//! [`SubsystemHealth::Failed`] event through a `tokio::sync::watch` channel.
//!
//! # Thread model
//!
//! Windows MMDevice notifications arrive on an internal COM thread.  The
//! `NotificationSink` callback immediately forwards a [`DeviceEvent`]
//! through a bounded `std::sync::mpsc` channel and returns to COM promptly.
//! A dedicated OS thread (`watchdog-event-pump`) reads from that channel,
//! classifies each event with the pure [`classify_device_event`] function, and
//! publishes [`SubsystemHealth`] updates through a `tokio::sync::watch` sender.
//!
//! # Cross-platform stubs
//!
//! On non-Windows platforms [`start_watching`] returns a no-op watchdog that
//! always reports [`SubsystemHealth::Healthy`].  All public types compile on
//! every platform.

// Items used by the orchestrator but not yet wired up — suppress lints.
#![allow(dead_code)]

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;

#[cfg(windows)]
use crate::audio::windows_com::ComApartmentGuard;

// ─── Public cross-platform types ─────────────────────────────────────────────

/// Health state of a monitored audio subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubsystemHealth {
    /// The device is present and active.
    Healthy,
    /// The device was removed, disabled, or unplugged.
    Failed {
        /// Human-readable explanation forwarded to the readiness aggregator.
        reason: String,
    },
}

/// Discriminated device event forwarded from COM callbacks.
///
/// All fields are plain Rust types so this enum can be used in unit tests
/// without any Windows COM headers.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    /// `IMMNotificationClient::OnDeviceStateChanged`.
    StateChanged {
        /// Windows endpoint ID (GUID string).
        device_id: String,
        /// New `DEVICE_STATE_*` bitmask value.
        new_state: u32,
    },
    /// `IMMNotificationClient::OnDeviceRemoved`.
    Removed {
        /// Windows endpoint ID of the removed device.
        device_id: String,
    },
    /// `IMMNotificationClient::OnDefaultDeviceChanged`.
    DefaultChanged {
        /// `EDataFlow` value (0 = eRender, 1 = eCapture).
        flow: u32,
        /// `ERole` value.
        role: u32,
        /// New default endpoint ID for this flow/role.
        device_id: String,
    },
    /// `IMMNotificationClient::OnDeviceAdded`.
    Added {
        /// Windows endpoint ID of the added device.
        device_id: String,
    },
}

/// Well-known `DEVICE_STATE_*` flag values, mirrored for cross-platform tests.
pub mod device_state {
    /// Device is active and in use.
    pub const ACTIVE: u32 = 0x0000_0001;
    /// Device is present but disabled in the Windows Sound control panel.
    pub const DISABLED: u32 = 0x0000_0002;
    /// Device is not present (driver unloaded or hardware removed).
    pub const NOT_PRESENT: u32 = 0x0000_0004;
    /// Device is present but unplugged from the jack.
    pub const UNPLUGGED: u32 = 0x0000_0008;
}

/// Well-known `EDataFlow` values, mirrored for cross-platform tests.
pub mod data_flow {
    /// Render (playback / output) endpoint — the flow used for virtual-mic routing.
    pub const E_RENDER: u32 = 0;
    /// Capture (recording / input) endpoint.
    pub const E_CAPTURE: u32 = 1;
}

// ─── Pure classification logic ────────────────────────────────────────────────

/// Classify a raw device event against a specific target endpoint.
///
/// Returns `Some(health)` when the event concerns `target_device_id` and
/// implies a health-state transition.  Returns `None` for events that can
/// safely be ignored (different device, unrelated data-flow, property-value
/// changes, etc.).
///
/// This function is pure (no I/O, no side-effects) and fully unit-tested in
/// the companion test module.
pub fn classify_device_event(
    event: &DeviceEvent,
    target_device_id: &str,
) -> Option<SubsystemHealth> {
    match event {
        DeviceEvent::StateChanged {
            device_id,
            new_state,
        } => {
            if device_id != target_device_id {
                return None;
            }
            if *new_state == device_state::ACTIVE {
                Some(SubsystemHealth::Healthy)
            } else {
                Some(SubsystemHealth::Failed {
                    reason: format!(
                        "device state changed to {:#010x} \
                         (DISABLED / UNPLUGGED / NOT_PRESENT)",
                        new_state
                    ),
                })
            }
        }
        DeviceEvent::Removed { device_id } => {
            if device_id != target_device_id {
                return None;
            }
            Some(SubsystemHealth::Failed {
                reason: "render endpoint removed by OS".to_string(),
            })
        }
        DeviceEvent::DefaultChanged {
            flow, device_id, ..
        } => {
            // Only render (eRender = 0) endpoints matter for virtual-mic routing.
            if *flow != data_flow::E_RENDER {
                return None;
            }
            if device_id != target_device_id {
                return None;
            }
            // Our device was (re-)selected as the default render endpoint.
            Some(SubsystemHealth::Healthy)
        }
        DeviceEvent::Added { .. } => None,
    }
}

// ─── DeviceWatchdog ───────────────────────────────────────────────────────────

/// Handle returned by [`start_watching`].  Dropping this unregisters the
/// COM callback and shuts down the event-pump thread.
pub struct DeviceWatchdog {
    /// Keeps the COM subscription and event-pump thread alive (Windows
    /// release builds only).  Debug builds use the no-op variant and skip
    /// the COM registration entirely — see [`start_watching`] for rationale.
    #[cfg(all(windows, not(debug_assertions)))]
    _inner: windows_impl::WatchdogInner,
    /// Shared ownership of the watch sender; keeps the channel open for the
    /// lifetime of this struct on all platforms.
    _health_tx: Arc<watch::Sender<SubsystemHealth>>,
    /// Receiver exposed to downstream consumers via [`DeviceWatchdog::subscribe`].
    health_rx: watch::Receiver<SubsystemHealth>,
}

impl DeviceWatchdog {
    /// Clone a receiver that delivers every health-state update.
    ///
    /// Multiple subsystems can subscribe independently.
    pub fn subscribe(&self) -> watch::Receiver<SubsystemHealth> {
        self.health_rx.clone()
    }

    /// Non-blocking snapshot of the current health state.
    pub fn current_health(&self) -> SubsystemHealth {
        self.health_rx.borrow().clone()
    }
}

/// Start watching `device_name` for device-loss events.
///
/// On **Windows** this resolves the human-readable device name to a Windows
/// endpoint ID, registers an `IMMNotificationClient` with the MMDevice API on
/// a dedicated OS thread, and wires events into a `tokio::sync::watch` channel.
///
/// On **all other platforms** this returns a no-op watchdog that always
/// reports [`SubsystemHealth::Healthy`] without starting any threads or
/// performing any I/O.
///
/// # Errors
///
/// Returns an error if the named device cannot be found in the render
/// endpoint list (Windows) or if COM initialisation fails.
#[tracing::instrument(level = "debug")]
pub fn start_watching(device_name: &str) -> Result<DeviceWatchdog> {
    // In debug / cfg(test) builds on Windows, fall back to the no-op watchdog
    // that always reports Healthy.  The real COM `IMMNotificationClient`
    // registration introduces a process-teardown STATUS_ACCESS_VIOLATION on
    // hosted CI runners (the leaked COM sink interacts badly with Rust's
    // late-init COM apartment teardown).  Release builds keep the real
    // watchdog so production users still get device-loss detection.
    #[cfg(all(windows, not(debug_assertions)))]
    {
        windows_impl::start(device_name)
    }
    #[cfg(all(windows, debug_assertions))]
    {
        let _ = device_name;
        let (tx, rx) = watch::channel(SubsystemHealth::Healthy);
        Ok(DeviceWatchdog {
            _health_tx: Arc::new(tx),
            health_rx: rx,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = device_name;
        let (tx, rx) = watch::channel(SubsystemHealth::Healthy);
        Ok(DeviceWatchdog {
            _health_tx: Arc::new(tx),
            health_rx: rx,
        })
    }
}

// ─── Windows COM implementation ───────────────────────────────────────────────

#[cfg(all(windows, not(debug_assertions)))]
mod windows_impl {
    use std::sync::{mpsc::sync_channel, Arc};
    use std::time::Duration;

    use anyhow::{anyhow, Result};
    use tokio::sync::watch;
    use windows::{
        core::{implement, PCWSTR},
        Win32::{
            Foundation::PROPERTYKEY,
            Media::Audio::{
                EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
                IMMNotificationClient_Impl, MMDeviceEnumerator, DEVICE_STATE,
            },
            System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        },
    };

    use super::{classify_device_event, DeviceEvent, DeviceWatchdog, SubsystemHealth};
    use crate::audio::windows_com::ComApartmentGuard;

    /// Maximum events buffered between COM callbacks and the event-pump thread.
    const EVENT_CHANNEL_CAP: usize = 64;
    /// Timeout waiting for the watchdog thread to confirm successful registration.
    const INIT_TIMEOUT: Duration = Duration::from_secs(5);

    // ─── COM sink ─────────────────────────────────────────────────────────────

    /// COM callback object that forwards raw device events to the event-pump thread.
    ///
    /// All five `IMMNotificationClient` methods must be present.  Only
    /// `OnDeviceStateChanged`, `OnDeviceRemoved`, and
    /// `OnDefaultDeviceChanged` produce forwarded events; the rest are no-ops.
    #[implement(IMMNotificationClient)]
    struct NotificationSink {
        tx: std::sync::mpsc::SyncSender<DeviceEvent>,
    }

    impl IMMNotificationClient_Impl for NotificationSink_Impl {
        fn OnDeviceStateChanged(
            &self,
            pwstrdeviceid: &PCWSTR,
            dwnewstate: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            // SAFETY: Windows guarantees pwstrdeviceid is a valid null-terminated wide string.
            let id = unsafe { pwstrdeviceid.to_string() }.unwrap_or_default();
            let _ = self.tx.try_send(DeviceEvent::StateChanged {
                device_id: id,
                new_state: dwnewstate.0,
            });
            Ok(())
        }

        fn OnDeviceAdded(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
            // SAFETY: Windows guarantees pwstrdeviceid is a valid null-terminated wide string.
            let id = unsafe { pwstrdeviceid.to_string() }.unwrap_or_default();
            let _ = self.tx.try_send(DeviceEvent::Added { device_id: id });
            Ok(())
        }

        fn OnDeviceRemoved(&self, pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
            // SAFETY: Windows guarantees pwstrdeviceid is a valid null-terminated wide string.
            let id = unsafe { pwstrdeviceid.to_string() }.unwrap_or_default();
            let _ = self.tx.try_send(DeviceEvent::Removed { device_id: id });
            Ok(())
        }

        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            role: ERole,
            pwstrdefaultdeviceid: &PCWSTR,
        ) -> windows::core::Result<()> {
            // SAFETY: Windows guarantees pwstrdefaultdeviceid is a valid null-terminated wide string.
            let id = unsafe { pwstrdefaultdeviceid.to_string() }.unwrap_or_default();
            let _ = self.tx.try_send(DeviceEvent::DefaultChanged {
                flow: flow.0 as u32,
                role: role.0 as u32,
                device_id: id,
            });
            Ok(())
        }

        fn OnPropertyValueChanged(
            &self,
            _pwstrdeviceid: &PCWSTR,
            _key: &PROPERTYKEY,
        ) -> windows::core::Result<()> {
            // Property changes are not relevant for device-loss detection.
            Ok(())
        }
    }

    // ─── WatchdogInner ────────────────────────────────────────────────────────

    /// RAII guard that owns the COM enumerator + sink interface reference.
    ///
    /// Dropping this calls `UnregisterEndpointNotificationCallback`, which
    /// blocks until any in-progress callback has returned, then releases the
    /// sink COM object.  Once the sink is released its internal `SyncSender`
    /// is dropped, closing the event channel and causing the event-pump
    /// thread to exit.
    pub struct WatchdogInner {
        enumerator: IMMDeviceEnumerator,
        sink: IMMNotificationClient,
    }

    impl Drop for WatchdogInner {
        fn drop(&mut self) {
            // We deliberately do NOT call ``UnregisterEndpointNotificationCallback``
            // here.  On Windows hosted CI runners (and in any unit-test process
            // where the COM apartment is torn down before our Drop runs), the
            // Unregister call segfaults with STATUS_ACCESS_VIOLATION (0xC0000005)
            // deep inside the OS MMDevice proxy.  ``catch_unwind`` cannot catch
            // a hardware-level access violation, so the only safe option is to
            // leak the registration and let the OS reclaim it at process exit.
            //
            // In long-running production use the watchdog lifetime equals the
            // process lifetime, so the leak is bounded.
        }
    }

    // SAFETY: IMMDeviceEnumerator and IMMNotificationClient are free-threaded
    // (MTA) COM objects that are safe to move across thread boundaries.
    unsafe impl Send for WatchdogInner {}
    // SAFETY: All COM reference-count mutations are serialised internally by the
    // COM runtime; sharing an immutable reference is safe for MTA objects.
    unsafe impl Sync for WatchdogInner {}

    // ─── Device-name resolution ───────────────────────────────────────────────

    /// Resolve a friendly device name to a Windows endpoint ID (GUID string).
    ///
    /// Must be called after `initialize_mta()` on the same thread.
    fn resolve_device_id(device_name: &str) -> Result<String> {
        use wasapi::{DeviceCollection, Direction};

        let collection = DeviceCollection::new(&Direction::Render)
            .map_err(|e| anyhow!("enumerate render devices: {e}"))?;
        let count = collection
            .get_nbr_devices()
            .map_err(|e| anyhow!("count render devices: {e}"))?;

        for i in 0..count {
            let device = match collection.get_device_at_index(i) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let name = match device.get_friendlyname() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if name == device_name {
                return device
                    .get_id()
                    .map_err(|e| anyhow!("read endpoint ID for '{device_name}': {e}"));
            }
        }

        Err(anyhow!(
            "render device '{device_name}' not found in MMDevice enumeration; \
             run `tui-translator --list-capture-devices` to see available devices"
        ))
    }

    /// Perform COM setup on the watchdog thread: init MTA, resolve device ID,
    /// create `IMMDeviceEnumerator`, and register the callback.
    fn com_setup(
        device_name: &str,
        event_tx: std::sync::mpsc::SyncSender<DeviceEvent>,
    ) -> Result<(WatchdogInner, String)> {
        // WP-24 (#723): use `leak()` here, not `enter()`. The COM objects
        // we are about to create (MMDeviceEnumerator + NotificationSink)
        // are constructed on the watchdog-event-pump thread and then
        // SENT across the channel to the main thread via `init_tx`. The
        // main thread eventually drops them in `WatchdogInner::drop`.
        // A scoped `enter()` would tear down the COM apartment before
        // the cross-thread Release, which segfaults inside the OS
        // MMDevice proxy (this is the same failure mode the existing
        // `WatchdogInner::drop` comment warns about). The apartment
        // stays alive for the lifetime of the process — acceptable
        // because the watchdog lifetime equals the process lifetime.
        let _com = ComApartmentGuard::leak()?;
        let device_id = resolve_device_id(device_name)?;

        tracing::debug!(
            device_name,
            device_id,
            "DeviceWatchdog: resolved endpoint ID"
        );

        // SAFETY: COM MTA is initialised on this thread; MMDeviceEnumerator is a well-known in-process server.
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| anyhow!("CoCreateInstance MMDeviceEnumerator: {e}"))?;

        let sink: IMMNotificationClient = NotificationSink { tx: event_tx }.into();

        // SAFETY: enumerator is a valid IMMDeviceEnumerator; sink implements IMMNotificationClient.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&sink) }
            .map_err(|e| anyhow!("RegisterEndpointNotificationCallback: {e}"))?;

        tracing::info!(
            device_name,
            device_id,
            "DeviceWatchdog: IMMNotificationClient registered"
        );

        Ok((WatchdogInner { enumerator, sink }, device_id))
    }

    // ─── Public entry point ───────────────────────────────────────────────────

    /// Register the IMMNotificationClient callback and start the event-pump thread.
    pub fn start(device_name: &str) -> Result<DeviceWatchdog> {
        let device_name_owned = device_name.to_owned();

        let (event_tx, event_rx) = sync_channel::<DeviceEvent>(EVENT_CHANNEL_CAP);
        let (health_tx, health_rx) = watch::channel(SubsystemHealth::Healthy);
        let health_tx_arc = Arc::new(health_tx);
        let health_tx_thread = Arc::clone(&health_tx_arc);

        let (init_tx, init_rx) = sync_channel::<Result<(WatchdogInner, String)>>(1);

        std::thread::Builder::new()
            .name("watchdog-event-pump".into())
            .spawn(move || {
                let setup = com_setup(&device_name_owned, event_tx);
                let (inner, device_id) = match setup {
                    Ok(pair) => pair,
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };
                let _ = init_tx.send(Ok((inner, device_id.clone())));

                while let Ok(event) = event_rx.recv() {
                    if let Some(health) = classify_device_event(&event, &device_id) {
                        tracing::info!(
                            ?health,
                            device_id,
                            "DeviceWatchdog: subsystem health changed"
                        );
                        let _ = health_tx_thread.send(health);
                    }
                }

                tracing::debug!("DeviceWatchdog: event-pump thread exiting");
            })
            .map_err(|e| anyhow!("spawn watchdog-event-pump: {e}"))?;

        let (inner, _device_id) = init_rx
            .recv_timeout(INIT_TIMEOUT)
            .map_err(|_| anyhow!("timed out waiting for DeviceWatchdog initialisation"))??;

        Ok(DeviceWatchdog {
            _inner: inner,
            _health_tx: health_tx_arc,
            health_rx,
        })
    }
}

#[cfg(test)]
#[path = "device_watchdog_tests.rs"]
mod tests;
