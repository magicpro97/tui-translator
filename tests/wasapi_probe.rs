//! Windows WASAPI loopback probe — issue #32 honest-proof path.
//!
//! Verifies that the default Windows render endpoint can be opened via the
//! WASAPI API and that the mix format is queryable.  This is the minimum
//! environmental prerequisite for running WASAPI loopback capture.
//!
//! Run with:
//!   cargo test --test wasapi_probe -- --nocapture
//!
//! On non-Windows CI all tests in this file are compiled away. On Windows
//! hosts with no default playback device, the probe logs the missing
//! prerequisite and skips instead of failing unrelated test runs.

#[cfg(windows)]
mod wasapi_probe {
    use wasapi::{get_default_device, Device, DeviceCollection, DeviceState, Direction};

    // WP-24 (#723): integration tests cannot `use tui_translator::...`
    // because this crate is binary-only (no [lib] target). Use a
    // hand-rolled RAII guard inline that matches the production
    // `ComApartmentGuard` semantics for the test's lifetime.
    //
    // We deliberately inline rather than share with the production
    // module: integration tests compile in their own crate and can
    // only reach `pub` items from the library, but this crate has
    // no library target (see Cargo.toml — only `[[bin]]` entries).
    mod com_apartment_guard {
        use wasapi::initialize_mta;

        pub struct Guard {
            owns_apartment: bool,
        }

        pub fn enter() -> Result<Guard, ()> {
            match initialize_mta() {
                Ok(()) => Ok(Guard {
                    owns_apartment: true,
                }),
                Err(_) => Ok(Guard {
                    owns_apartment: false,
                }),
            }
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                if self.owns_apartment {
                    wasapi::deinitialize();
                }
            }
        }
    }

    use self::com_apartment_guard::Guard as ComApartmentGuard;

    /// WP-24 (#723): RAII-based COM init. Returns `Some(guard)` on
    /// success (the guard balances the per-thread COM ref count on
    /// Drop) or `None` on headless / no-COM hosts (test body skips).
    fn com_apartment_or_skip() -> Option<ComApartmentGuard> {
        match com_apartment_guard::enter() {
            Ok(g) => Some(g),
            Err(()) => {
                eprintln!("[wasapi-probe] skipping: COM MTA initialization failed");
                None
            }
        }
    }

    fn default_render_device_or_skip() -> Option<Device> {
        match get_default_device(&Direction::Render) {
            Ok(device) => Some(device),
            Err(err) => {
                eprintln!(
                    "[wasapi-probe] skipping: no default playback device is registered: {err}"
                );
                None
            }
        }
    }

    /// Verify that the default render endpoint can be discovered.
    ///
    /// Failure means no default render device is registered in Windows — the
    /// WASAPI loopback module will fail at runtime with the same error.
    #[test]
    fn default_render_endpoint_opens() {
        let Some(_com) = com_apartment_or_skip() else {
            return;
        };

        let Some(device) = default_render_device_or_skip() else {
            return;
        };

        let name = device
            .get_friendlyname()
            .unwrap_or_else(|_| "unknown".into());
        let id = device
            .get_id()
            .expect("default render device must expose a stable endpoint id");

        eprintln!("[wasapi-probe] default render device: {name} ({id})");
        assert!(!name.is_empty(), "device name must not be empty");
        assert!(!id.is_empty(), "device endpoint id must not be empty");
    }

    /// Verify active render endpoint enumeration exposes stable identity and
    /// user-facing display names while filtering devices that cannot be opened.
    #[test]
    fn active_render_endpoint_enumeration_exposes_stable_ids() {
        let Some(_com) = com_apartment_or_skip() else {
            return;
        };

        let collection = DeviceCollection::new(&Direction::Render)
            .expect("active render endpoint enumeration must succeed");
        let count = collection
            .get_nbr_devices()
            .expect("active render endpoint count must be readable");
        if count == 0 {
            eprintln!("[wasapi-probe] skipping: no active render endpoints were reported");
            return;
        }

        let mut usable_count = 0;
        for index in 0..count {
            let device = collection
                .get_device_at_index(index)
                .expect("active render endpoint must be readable");
            let state = device
                .get_state()
                .expect("active render endpoint state must be readable");
            let name = device
                .get_friendlyname()
                .expect("active render endpoint display name must be readable");
            let id = device
                .get_id()
                .expect("active render endpoint stable id must be readable");
            if let Err(err) = device
                .get_iaudioclient()
                .and_then(|client| client.get_mixformat().map(|_| ()))
            {
                eprintln!("[wasapi-probe] skipping unusable active render endpoint {index}: {err}");
                continue;
            }

            eprintln!("[wasapi-probe] active render endpoint {index}: {name} ({id})");
            assert_eq!(state, DeviceState::Active, "endpoint must be active");
            assert!(!name.is_empty(), "endpoint display name must not be empty");
            assert!(!id.is_empty(), "endpoint stable id must not be empty");
            usable_count += 1;
        }

        if usable_count == 0 {
            eprintln!(
                "[wasapi-probe] skipping: no active render endpoint had a queryable mix format"
            );
        }
    }

    /// Verify that the mix format (native PCM geometry) can be queried.
    ///
    /// The capture loop in `wasapi_capture.rs` calls `get_mixformat()` immediately
    /// after opening the device.  If this fails the module cannot initialise.
    #[test]
    fn default_render_endpoint_format_is_queryable() {
        let Some(_com) = com_apartment_or_skip() else {
            return;
        };

        let Some(device) = default_render_device_or_skip() else {
            return;
        };

        let audio_client = device
            .get_iaudioclient()
            .expect("IAudioClient must be obtainable from the render device");

        let wave_fmt = audio_client
            .get_mixformat()
            .expect("get_mixformat must succeed on the default render device");

        let channels = wave_fmt.get_nchannels();
        let sample_rate = wave_fmt.get_samplespersec();
        let bits = wave_fmt.get_bitspersample();

        eprintln!(
            "[wasapi-probe] mix format: channels={channels}, \
             sample_rate={sample_rate} Hz, bits={bits}"
        );

        assert!(channels > 0, "channel count must be > 0");
        assert!(sample_rate > 0, "sample rate must be > 0");
        assert!(bits > 0, "bit depth must be > 0");
        // The capture path currently handles 16-bit samples and 32-bit sample
        // containers; the 32-bit path is decoded as f32 samples in
        // wasapi_capture.rs.
        assert!(
            bits == 16 || bits == 32,
            "bit depth {bits} is not handled by the capture module; \
             only 16-bit and 32-bit sample containers are supported"
        );
    }

    /// Verify that the audio client can be initialised in shared loopback mode.
    ///
    /// This matches the exact `initialize_client` call in `wasapi_capture.rs`
    /// and is the last step before the capture loop starts reading frames.
    #[test]
    fn default_render_endpoint_initialises_for_loopback() {
        use wasapi::ShareMode;
        let Some(_com) = com_apartment_or_skip() else {
            return;
        };

        let Some(device) = default_render_device_or_skip() else {
            return;
        };

        let mut audio_client = device
            .get_iaudioclient()
            .expect("IAudioClient must be obtainable");

        let wave_fmt = audio_client
            .get_mixformat()
            .expect("get_mixformat must succeed");

        let (_def_period, min_period) = audio_client
            .get_periods()
            .expect("get_periods must succeed");

        audio_client
            .initialize_client(
                &wave_fmt,
                min_period,
                &Direction::Capture,
                &ShareMode::Shared,
                true, // loopback = true
            )
            .expect(
                "initialize_client(loopback=true) must succeed — \
                 WASAPI loopback capture cannot start without this",
            );

        eprintln!("[wasapi-probe] loopback client initialised successfully");
    }
}
