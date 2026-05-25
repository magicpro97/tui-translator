//! Runtime MT router that resolves a language pair to a concrete provider
//! action and dispatches accordingly (JV-12, issue #420).
//!
//! The router is the runtime counterpart of [`super::routing`]: that module
//! defines the pure routing decision and resolution; this module wires the
//! result into actual provider calls while preserving the no-silent-cloud
//! invariant.
//!
//! # Contract
//!
//! For every `translate(text, source, target)` call:
//!
//! 1. Build a [`LanguagePair`] from the source/target tags (BCP-47 normalised).
//! 2. Resolve the route via [`route_for_pair`] + [`resolve`].
//! 3. Dispatch:
//!    * [`ResolvedRoute::LocalDirect`] → call the local provider.
//!    * [`ResolvedRoute::CloudFallback`] → call the cloud provider (only
//!      when the router was constructed with an explicit cloud fallback;
//!      see [`MtRouter::new`]).  Emit a single `tracing::warn!` so the
//!      privacy boundary crossing is auditable.
//!    * [`ResolvedRoute::Unsupported`] or [`ResolvedRoute::LocalPivotPlanned`]
//!      → return the provider error produced by
//!      [`ResolvedRoute::as_provider_error`] **without** touching the
//!      cloud provider, even if one is configured.
//!
//! # No-silent-cloud invariant
//!
//! The router holds the cloud provider in an `Option<C>` that is only `Some`
//! when the operator explicitly set `mt_cloud_fallback = "google"` in
//! `config.json` **and** the corresponding API key is available.  Key
//! presence alone (without `mt_cloud_fallback`) leaves the cloud slot empty
//! and the router treats every unsupported pair as
//! [`ResolvedRoute::Unsupported`].

use crate::providers::mt::routing::{resolve, route_for_pair, LanguagePair, ResolvedRoute};
use crate::providers::{MtProvider, MtResult, ProviderError};

/// Routes MT calls to a local provider or an optional explicit cloud
/// fallback, based on the [`super::routing`] decision table.
///
/// Generic so unit tests can substitute mock providers for both legs.
#[derive(Debug)]
pub struct MtRouter<L: MtProvider, C: MtProvider> {
    local: L,
    cloud_fallback: Option<C>,
}

impl<L: MtProvider, C: MtProvider> MtRouter<L, C> {
    /// Construct a router with a local primary and an optional explicit
    /// cloud fallback.
    ///
    /// `cloud_fallback` MUST be `Some(_)` only when both
    /// `mt_cloud_fallback = "google"` is explicitly configured **and** the
    /// corresponding API key is available.  The caller is responsible for
    /// enforcing the consent contract at construction time.
    pub fn new(local: L, cloud_fallback: Option<C>) -> Self {
        Self {
            local,
            cloud_fallback,
        }
    }

    /// `true` when an explicit cloud fallback is wired in.
    pub fn has_cloud_fallback(&self) -> bool {
        self.cloud_fallback.is_some()
    }

    /// Resolve the route for a given pair using the router's current cloud
    /// fallback consent state.  Exposed for tests and TUI status code.
    pub fn resolve_pair(&self, pair: &LanguagePair) -> ResolvedRoute {
        let decision = route_for_pair(pair);
        resolve(&decision, self.has_cloud_fallback())
    }
}

impl<L: MtProvider, C: MtProvider> MtProvider for MtRouter<L, C> {
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "mt-router"))]
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        let pair = LanguagePair::new(source_language, target_language);
        let decision = route_for_pair(&pair);
        let route = resolve(&decision, self.has_cloud_fallback());

        match route {
            ResolvedRoute::LocalDirect => {
                self.local
                    .translate(text, source_language, target_language)
                    .await
            }
            ResolvedRoute::CloudFallback => {
                // `has_cloud_fallback()` guarantees `cloud_fallback` is Some,
                // so this branch is reachable only when consent + key are
                // present at construction time.
                let cloud = self.cloud_fallback.as_ref().expect(
                    "ResolvedRoute::CloudFallback requires cloud provider to be Some; \
                     this is an MtRouter contract bug",
                );
                tracing::warn!(
                    source = %pair.source,
                    target = %pair.target,
                    "MT routed via cloud fallback (no local model for pair, \
                     mt_cloud_fallback=\"google\")"
                );
                cloud
                    .translate(text, source_language, target_language)
                    .await
            }
            ResolvedRoute::Unsupported | ResolvedRoute::LocalPivotPlanned => {
                Err(route.as_provider_error(&pair).unwrap_or_else(|| {
                    ProviderError::InvalidInput(format!(
                        "unsupported MT route for {}→{}",
                        pair.source, pair.target
                    ))
                }))
            }
        }
    }
}

// Convenience: callers who want the resolved-route classification for a
// decision without constructing a router can keep using
// `routing::route_for_pair` + `routing::resolve` directly.

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    type TestRouter = MtRouter<Arc<CountingMt>, Arc<CountingMt>>;

    #[derive(Debug, Default)]
    struct CountingMt {
        calls: AtomicUsize,
        label: &'static str,
    }

    impl CountingMt {
        fn new(label: &'static str) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                label,
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl MtProvider for Arc<CountingMt> {
        async fn translate(
            &self,
            text: &str,
            _source_language: &str,
            _target_language: &str,
        ) -> Result<MtResult, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(MtResult {
                translated_text: format!("[{}] {}", self.label, text),
                detected_source_language: Some("xx".to_string()),
            })
        }
    }

    fn router_with_fallback() -> (TestRouter, Arc<CountingMt>, Arc<CountingMt>) {
        let local = CountingMt::new("local");
        let cloud = CountingMt::new("cloud");
        let router = MtRouter::new(local.clone(), Some(cloud.clone()));
        (router, local, cloud)
    }

    fn router_without_fallback() -> (TestRouter, Arc<CountingMt>) {
        let local = CountingMt::new("local");
        let router = MtRouter::new(local.clone(), None);
        (router, local)
    }

    #[tokio::test]
    async fn ja_vi_routes_to_local_direct_no_cloud_call() {
        let (router, local, cloud) = router_with_fallback();
        let result = router.translate("こんにちは", "ja-JP", "vi").await.unwrap();
        assert!(result.translated_text.starts_with("[local]"));
        assert_eq!(local.calls(), 1, "local must be called for ja→vi");
        assert_eq!(
            cloud.calls(),
            0,
            "cloud must NOT be called when local route covers the pair"
        );
    }

    #[tokio::test]
    async fn unsupported_pair_without_fallback_returns_invalid_input_no_calls() {
        let (router, local) = router_without_fallback();
        let err = router
            .translate("hello", "ko", "vi")
            .await
            .expect_err("unsupported pair without fallback must error");
        assert!(
            matches!(err, ProviderError::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("mt_cloud_fallback"),
            "error must point operator at consent knob; got: {msg}"
        );
        assert_eq!(
            local.calls(),
            0,
            "local must NOT be called for unsupported pair"
        );
    }

    #[tokio::test]
    async fn unsupported_pair_with_fallback_calls_cloud_only() {
        let (router, local, cloud) = router_with_fallback();
        let result = router.translate("hello", "ko", "vi").await.unwrap();
        assert!(result.translated_text.starts_with("[cloud]"));
        assert_eq!(
            local.calls(),
            0,
            "local must NOT be called for unsupported pair"
        );
        assert_eq!(cloud.calls(), 1, "cloud must be called once");
    }

    /// Key-presence-alone must NOT enable cloud fallback.  In the router this
    /// corresponds to constructing it with `cloud_fallback = None` regardless
    /// of any key the surrounding application may hold.
    #[tokio::test]
    async fn key_presence_alone_does_not_enable_cloud() {
        // Simulate: caller has a key but did NOT set mt_cloud_fallback —
        // the contract says the router must be constructed with None.
        let (router, local) = router_without_fallback();
        // The router is unaware of any key; an unsupported pair must error.
        let err = router.translate("hello", "ko", "vi").await.unwrap_err();
        assert!(matches!(err, ProviderError::InvalidInput(_)));
        assert_eq!(local.calls(), 0);
    }

    #[tokio::test]
    async fn resolve_pair_reflects_fallback_state() {
        let (router_yes, _, _) = router_with_fallback();
        assert_eq!(
            router_yes.resolve_pair(&LanguagePair::new("ko", "vi")),
            ResolvedRoute::CloudFallback
        );
        assert_eq!(
            router_yes.resolve_pair(&LanguagePair::new("ja", "vi")),
            ResolvedRoute::LocalDirect
        );

        let (router_no, _) = router_without_fallback();
        assert_eq!(
            router_no.resolve_pair(&LanguagePair::new("ko", "vi")),
            ResolvedRoute::Unsupported
        );
    }

    #[tokio::test]
    async fn region_subtags_normalised_before_routing() {
        let (router, local, cloud) = router_with_fallback();
        router
            .translate("こんにちは", "JA-JP", "VI-VN")
            .await
            .unwrap();
        assert_eq!(local.calls(), 1);
        assert_eq!(cloud.calls(), 0);
    }
}
