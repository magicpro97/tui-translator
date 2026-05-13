//! Windows WASAPI loopback probe — issue #32 honest-proof path.
//!
//! Verifies that the default Windows render endpoint can be opened via the
//! WASAPI API and that the mix format is queryable.  This is the minimum
//! environmental prerequisite for running WASAPI loopback capture.
//!
//! Run with:
//!   cargo test --test wasapi_probe -- --nocapture
//!
//! On non-Windows CI all tests in this file are compiled away.

#[cfg(windows)]
mod wasapi_probe {
    use wasapi::{get_default_device, initialize_mta, Direction};

    /// Verify that the default render endpoint can be discovered.
    ///
    /// Failure means no default render device is registered in Windows — the
    /// WASAPI loopback module will fail at runtime with the same error.
    #[test]
    fn default_render_endpoint_opens() {
        initialize_mta().expect("COM MTA initialization must succeed");

        let device = get_default_device(&Direction::Render)
            .expect("get_default_device(Render) must succeed — is a default playback device set?");

        let name = device
            .get_friendlyname()
            .unwrap_or_else(|_| "unknown".into());

        eprintln!("[wasapi-probe] default render device: {name}");
        assert!(!name.is_empty(), "device name must not be empty");
    }

    /// Verify that the mix format (native PCM geometry) can be queried.
    ///
    /// The capture loop in `wasapi_capture.rs` calls `get_mixformat()` immediately
    /// after opening the device.  If this fails the module cannot initialise.
    #[test]
    fn default_render_endpoint_format_is_queryable() {
        initialize_mta().expect("COM MTA initialization must succeed");

        let device = get_default_device(&Direction::Render)
            .expect("get_default_device(Render) must succeed");

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
        initialize_mta().expect("COM MTA initialization must succeed");

        let device = get_default_device(&Direction::Render)
            .expect("get_default_device(Render) must succeed");

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
