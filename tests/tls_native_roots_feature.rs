// Tests that gate the `rustls-tls-native-roots` reqwest feature (issue #719).
//
// These tests ensure the OS trust store is consulted in addition to the
// bundled Mozilla CA roots so that corporate/MITM-proxy networks don't
// cause UnknownIssuer failures.

/// Hard gate: Cargo.toml must declare `rustls-tls-native-roots` for the
/// reqwest dependency.  If this test ever fails it means someone accidentally
/// removed the feature and broke corp-network TLS support.
#[test]
fn reqwest_dependency_enables_native_roots() {
    let manifest =
        std::fs::read_to_string("Cargo.toml").expect("Cargo.toml must be readable from test cwd");
    let needle = "rustls-tls-native-roots";
    assert!(
        manifest.contains(needle),
        "Cargo.toml reqwest features must include `{needle}` so the HTTP client \
         honours OS-trusted corporate CAs (issue #719). \
         Current reqwest line:\n{}",
        manifest
            .lines()
            .find(|l| l.contains("reqwest"))
            .unwrap_or("<not found>")
    );
}

/// Regression guard: adding native-roots must NOT break reaching a host whose
/// certificate is signed by a standard Mozilla-trusted CA (huggingface.co).
///
/// Marked `#[ignore]` so routine `cargo test` runs don't depend on the
/// network.  Run explicitly in the release-gate CI job via:
///   `cargo test --test tls_native_roots_feature -- --ignored`
#[tokio::test]
#[ignore = "network: requires outbound HTTPS to huggingface.co"]
async fn client_built_with_native_roots_still_reaches_mozilla_host() {
    let client = reqwest::Client::builder()
        .user_agent("tui-translator-test")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("client build must succeed with native-roots feature enabled");
    let resp = client
        .head("https://huggingface.co/")
        .send()
        .await
        .expect("native-roots client must still reach a Mozilla-PKI host");
    assert!(
        resp.status().is_success() || resp.status().is_redirection(),
        "unexpected HTTP status from Mozilla-PKI host: {}",
        resp.status()
    );
}

/// Source-scan guardrail: no reqwest client constructor in production code
/// should bypass the workspace feature set by using bare `Client::new()`.
///
/// `Client::new()` and `Client::builder().build()` both honour the feature
/// flags, so this test is more about ensuring future contributors don't add a
/// per-client `tls_built_in_root_certs(false)` call that would silently
/// opt-out of native roots on corp networks.
///
/// The test scans every `.rs` file under `src/` and asserts that none of them
/// contain `.tls_built_in_root_certs(false)` (the reqwest 0.12 call that
/// explicitly disables native roots).
#[test]
fn no_production_code_disables_native_tls_roots() {
    let forbidden = "tls_built_in_root_certs(false)";
    let src_root = std::path::Path::new("src");
    let violations = collect_rust_sources(src_root)
        .into_iter()
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            if content.contains(forbidden) {
                Some(path.display().to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "Found `{forbidden}` in production source files — this disables native \
         OS-trust-store roots and would break corp-network TLS (#719):\n{}",
        violations.join("\n")
    );
}

fn collect_rust_sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(collect_rust_sources(&path));
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out
}
