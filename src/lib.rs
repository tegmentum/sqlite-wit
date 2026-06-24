//! Canonical Rust types for the `sqlite:extension/policy` WIT
//! contract.
//!
//! This crate is the single source of truth for `Capability`,
//! `HttpPolicy`, `Policy`, and `PolicyError`. Both loader-side hosts
//! (`sqlink-loader/runtimes/{wasmtime,wamr}`) and the in-WASM
//! host (`sqlink/host`) depend on it so a `Policy` value
//! constructed once is portable across deployment modes — a fact the
//! WIT contract advertises, but only this Rust crate enforces at the
//! type level.
//!
//! The crate intentionally has no `wasmtime` or `wit-bindgen-rt`
//! dependency. Bindgen-generated `LoadOptions` / `Capability` /
//! `HttpPolicy` types are crate-local to each consumer — they would
//! otherwise force this crate to pick one runtime — so the
//! conversion lives at each consumer instead. The shape of the
//! conversion is identical everywhere, so each consumer's `from_wit`
//! is ~30 LOC: walk the WIT capability list, map each variant to
//! `Capability`, copy the numeric knobs across.

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

use std::collections::HashSet;

/// One capability per gated host-imported interface. Mirrors
/// `sqlite:extension/policy.capability` from the WIT.
///
/// Utility interfaces (`text`, `hashing`, `encoding`, `random`) are
/// represented here for transparency even though hosts will usually
/// grant them by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Spi,
    Prepared,
    Transaction,
    Schema,
    State,
    Cache,
    Random,
    Text,
    Hashing,
    Encoding,
    Http,
    Dns,
    /// Raw WAL-frame access via `sqlite:extension/wal-frames`.
    /// Substrate for the wal-archive extension; gated separately
    /// from `Spi` so an extension can read WAL bytes without
    /// holding the full SQL surface (and vice versa).
    WalFrames,
    /// S3-compatible object storage via `sqlite:extension/s3-base`.
    /// Substrate for the wal-archive extension's off-box sink
    /// (PLAN-wal-archive-extension.md #440). Endpoint URL +
    /// credentials are runtime arguments to each call rather than
    /// policy fields  the operator's grant is purely an
    /// allow-the-surface bit.
    S3,
}

/// Outbound HTTP policy. The host's `http::handle` impl consults
/// this before letting an extension hit the network.
///
/// Method names are unstructured strings (e.g. `"GET"`, `"POST"`)
/// rather than a `Method` enum so the type stays bindgen-free.
/// Consumers that use a runtime-specific `Method` enum should match
/// by string before/after the boundary.
#[derive(Debug, Clone, Default)]
pub struct HttpPolicy {
    /// Allowlist of hostnames. Entries may use a leading `*.` for a
    /// wildcard suffix (e.g. `*.example.com` matches
    /// `api.example.com` but not `example.com` itself). Empty = no
    /// hosts permitted.
    pub allowed_hosts: Vec<String>,
    /// Optional method allowlist. `None` = any method allowed.
    pub allowed_methods: Option<Vec<String>>,
    /// Cap on response body bytes returned to the extension.
    pub max_body_bytes: Option<u64>,
    /// Per-request wall-clock timeout. Overrides any timeout the
    /// extension passes.
    pub timeout_ms: Option<u32>,
}

impl HttpPolicy {
    /// True if `host` matches an allowlist entry (literal or
    /// `*.suffix` wildcard).
    pub fn allows(&self, host: &str) -> bool {
        for entry in &self.allowed_hosts {
            if let Some(suffix) = entry.strip_prefix("*.") {
                if host.ends_with(suffix) && host.len() > suffix.len() {
                    // Wildcard requires at least one character before
                    // the suffix.
                    return true;
                }
            } else if entry == host {
                return true;
            }
        }
        false
    }

    /// `Ok(())` if allowed; `Err(PolicyError::HostNotAllowed)`
    /// otherwise. Convenience wrapper over [`Self::allows`].
    pub fn check_host(&self, host: &str) -> Result<(), PolicyError> {
        if self.allows(host) {
            Ok(())
        } else {
            Err(PolicyError::HostNotAllowed(host.to_string()))
        }
    }

    /// `Ok(())` if `method` is in the allowlist (or no list set);
    /// `Err(PolicyError::MethodNotAllowed)` otherwise.
    pub fn check_method(&self, method: &str) -> Result<(), PolicyError> {
        match &self.allowed_methods {
            None => Ok(()),
            Some(list) if list.iter().any(|m| m.eq_ignore_ascii_case(method)) => Ok(()),
            Some(_) => Err(PolicyError::MethodNotAllowed(method.to_string())),
        }
    }
}

/// Outbound DNS policy. The host's `dns::resolve` impl consults
/// this before letting an extension issue a lookup.
#[derive(Debug, Clone, Default)]
pub struct DnsPolicy {
    /// Allowlist of domains. Entries may use a leading `*.` for a
    /// wildcard suffix (same semantics as [`HttpPolicy::allowed_hosts`]).
    /// Empty list = no domains permitted.
    pub allowed_domains: Vec<String>,
    /// Per-request wall-clock timeout. Overrides any timeout the
    /// extension passes.
    pub timeout_ms: Option<u32>,
}

impl DnsPolicy {
    /// True if `name` matches an allowlist entry (literal or `*.suffix`
    /// wildcard). Strips a trailing dot from `name` before matching so
    /// `example.com.` and `example.com` compare equal.
    pub fn allows(&self, name: &str) -> bool {
        let name = name.strip_suffix('.').unwrap_or(name);
        for entry in &self.allowed_domains {
            if let Some(suffix) = entry.strip_prefix("*.") {
                if name.ends_with(suffix) && name.len() > suffix.len() {
                    return true;
                }
            } else if entry == name {
                return true;
            }
        }
        false
    }

    /// `Ok(())` if allowed; `Err(PolicyError::DnsDomainNotAllowed)` otherwise.
    pub fn check_domain(&self, name: &str) -> Result<(), PolicyError> {
        if self.allows(name) {
            Ok(())
        } else {
            Err(PolicyError::DnsDomainNotAllowed(name.to_string()))
        }
    }
}

/// Full per-extension policy. Mirrors
/// `sqlite:extension/policy.load-options` plus a precomputed grant
/// set for fast lookups.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    granted: HashSet<Capability>,
    pub http: Option<HttpPolicy>,
    pub dns: Option<DnsPolicy>,
    pub fuel_per_call: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub epoch_deadline_ms: Option<u64>,
}

impl Policy {
    /// A Policy that grants nothing. Fail-closed default.
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Builder: grant the listed capabilities.
    pub fn with_grants(mut self, caps: impl IntoIterator<Item = Capability>) -> Self {
        self.granted.extend(caps);
        self
    }

    /// Builder: attach an HTTP policy. Required when `Capability::Http`
    /// is in the grant list — see [`Self::validate`].
    pub fn with_http(mut self, http: HttpPolicy) -> Self {
        self.http = Some(http);
        self
    }

    /// Builder: attach a DNS policy. Required when `Capability::Dns`
    /// is in the grant list — see [`Self::validate`].
    pub fn with_dns(mut self, dns: DnsPolicy) -> Self {
        self.dns = Some(dns);
        self
    }

    /// Builder: per-call fuel budget (wasmtime fuel reset before
    /// each guest invocation). `None` = unlimited.
    pub fn with_fuel_per_call(mut self, fuel: u64) -> Self {
        self.fuel_per_call = Some(fuel);
        self
    }

    /// Builder: hard cap on the extension's linear memory.
    pub fn with_memory_limit_bytes(mut self, n: u64) -> Self {
        self.memory_limit_bytes = Some(n);
        self
    }

    /// Builder: wall-clock deadline per guest invocation
    /// (milliseconds). Best-effort on runtimes that lack epoch
    /// interruption.
    pub fn with_epoch_deadline_ms(mut self, ms: u64) -> Self {
        self.epoch_deadline_ms = Some(ms);
        self
    }

    /// True if `cap` is in the grant list.
    pub fn is_granted(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }

    /// Return capabilities in `declared` that are NOT granted.
    /// Empty result ⇒ manifest is satisfiable.
    pub fn missing<'a>(&self, declared: &'a [Capability]) -> Vec<&'a Capability> {
        declared
            .iter()
            .filter(|c| !self.granted.contains(c))
            .collect()
    }

    /// `Ok(())` if every entry of `declared` is granted; otherwise
    /// `Err(PolicyError::CapabilityNotGranted(<first missing>))`.
    pub fn check_manifest(&self, declared: &[Capability]) -> Result<(), PolicyError> {
        for c in declared {
            if !self.granted.contains(c) {
                return Err(PolicyError::CapabilityNotGranted(*c));
            }
        }
        Ok(())
    }

    /// Cross-field validation. Run after construction to catch
    /// internally-inconsistent policies (e.g., http capability
    /// granted but no HttpPolicy attached).
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.granted.contains(&Capability::Http) && self.http.is_none() {
            return Err(PolicyError::MissingHttpPolicy);
        }
        if self.granted.contains(&Capability::Dns) && self.dns.is_none() {
            return Err(PolicyError::MissingDnsPolicy);
        }
        Ok(())
    }
}

/// Errors raised by the policy layer. Mirrors
/// `sqlite:extension/policy.policy-error` from the WIT.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyError {
    /// Extension declared a capability the host didn't grant.
    CapabilityNotGranted(Capability),
    /// Extension tried to use a capability it never declared.
    CapabilityNotDeclared(Capability),
    /// HTTP request to a host not on the allowlist.
    HostNotAllowed(String),
    /// HTTP request used a method outside `allowed_methods`.
    MethodNotAllowed(String),
    /// `grant` includes `Http` but `http_policy` was `None`.
    MissingHttpPolicy,
    /// DNS query targeted a domain outside `dns_policy.allowed_domains`.
    DnsDomainNotAllowed(String),
    /// `grant` includes `Dns` but `dns_policy` was `None`.
    MissingDnsPolicy,
    /// Per-call fuel budget exhausted.
    FuelExhausted,
    /// Memory cap reached.
    MemoryLimitExceeded,
    /// Wall-clock deadline reached.
    EpochDeadlineExceeded,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapabilityNotGranted(c) => write!(f, "capability {c:?} not granted"),
            Self::CapabilityNotDeclared(c) => write!(f, "capability {c:?} not declared"),
            Self::HostNotAllowed(h) => write!(f, "host {h:?} not on http allowlist"),
            Self::MethodNotAllowed(m) => write!(f, "method {m:?} not on http allowlist"),
            Self::MissingHttpPolicy => write!(f, "grant includes http but no http policy attached"),
            Self::DnsDomainNotAllowed(d) => write!(f, "domain {d:?} not on dns allowlist"),
            Self::MissingDnsPolicy => write!(f, "grant includes dns but no dns policy attached"),
            Self::FuelExhausted => write!(f, "per-call fuel exhausted"),
            Self::MemoryLimitExceeded => write!(f, "memory limit exceeded"),
            Self::EpochDeadlineExceeded => write!(f, "epoch deadline exceeded"),
        }
    }
}

impl std::error::Error for PolicyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_all_grants_nothing() {
        let p = Policy::deny_all();
        for c in [Capability::Spi, Capability::Http, Capability::Random, Capability::S3] {
            assert!(!p.is_granted(c));
        }
    }

    #[test]
    fn grants_set_membership() {
        let p = Policy::deny_all().with_grants([Capability::Spi, Capability::Http]);
        assert!(p.is_granted(Capability::Spi));
        assert!(p.is_granted(Capability::Http));
        assert!(!p.is_granted(Capability::State));
    }

    #[test]
    fn missing_returns_only_ungranted() {
        let p = Policy::deny_all().with_grants([Capability::Spi]);
        let declared = [Capability::Spi, Capability::State, Capability::Http];
        let missing = p.missing(&declared);
        assert_eq!(missing, vec![&Capability::State, &Capability::Http]);
    }

    #[test]
    fn check_manifest_succeeds_when_subset() {
        let p = Policy::deny_all().with_grants([Capability::Spi, Capability::Random]);
        assert!(p.check_manifest(&[Capability::Spi]).is_ok());
        assert!(p.check_manifest(&[]).is_ok());
    }

    #[test]
    fn check_manifest_errors_with_first_missing() {
        let p = Policy::deny_all().with_grants([Capability::Spi]);
        assert_eq!(
            p.check_manifest(&[Capability::Spi, Capability::Http]),
            Err(PolicyError::CapabilityNotGranted(Capability::Http))
        );
    }

    #[test]
    fn http_allows_exact_host() {
        let h = HttpPolicy {
            allowed_hosts: vec!["api.example.com".into()],
            ..Default::default()
        };
        assert!(h.allows("api.example.com"));
        assert!(!h.allows("api.other.com"));
    }

    #[test]
    fn http_wildcard_suffix() {
        let h = HttpPolicy {
            allowed_hosts: vec!["*.example.com".into()],
            ..Default::default()
        };
        assert!(h.allows("api.example.com"));
        assert!(h.allows("a.b.example.com"));
        // Wildcard requires at least one character before the suffix.
        assert!(!h.allows("example.com"));
        assert!(!h.allows("other.com"));
    }

    #[test]
    fn http_method_check() {
        let h = HttpPolicy {
            allowed_hosts: vec!["x.com".into()],
            allowed_methods: Some(vec!["GET".into(), "POST".into()]),
            ..Default::default()
        };
        assert!(h.check_method("GET").is_ok());
        assert!(h.check_method("get").is_ok()); // case-insensitive
        assert!(h.check_method("DELETE").is_err());
    }

    #[test]
    fn validate_catches_http_without_policy() {
        let p = Policy::deny_all().with_grants([Capability::Http]);
        assert_eq!(p.validate(), Err(PolicyError::MissingHttpPolicy));
    }
}
