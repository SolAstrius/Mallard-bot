//! `/dns` — sysadmin-grade DNS toolkit.
//!
//! Subcommands:
//!   * `/dns <name> [type] [@resolver]`        — basic lookup
//!   * `/dns prop <name> [type]`               — propagation table across resolvers
//!   * `/dns sec <name>`                       — DNSSEC chain validation
//!   * `/dns spf <domain>`                     — SPF expand + RFC-7208 lookup count
//!   * `/dns dmarc <domain>`                   — DMARC policy explainer
//!   * `/dns mx <domain>`                      — MX list with PTR + STARTTLS + DANE
//!   * `/dns mta <domain>`                     — MTA-STS policy fetch + validate
//!   * `/dns caa <name>`                       — CAA tree walk up the zone
//!
//! Resolver pool: six well-known public resolvers, individually toggleable
//! through `dns.resolvers.<name>` feature flags. Custom `@resolver` (an IP
//! or hostname) bypasses the pool entirely.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::{Resolver, TokioResolver};

use crate::features;

/// Named public resolver. Order here is the column order of the
/// propagation table.
pub struct Upstream {
    pub flag_leaf: &'static str, // matches dns.resolvers.<leaf>
    pub label: &'static str,
    pub ips: &'static [IpAddr],
}

pub const UPSTREAMS: &[Upstream] = &[
    Upstream {
        flag_leaf: "cloudflare",
        label: "Cloudflare",
        ips: &[
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(1, 0, 0, 1)),
        ],
    },
    Upstream {
        flag_leaf: "google",
        label: "Google",
        ips: &[
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4)),
        ],
    },
    Upstream {
        flag_leaf: "quad9",
        label: "Quad9",
        ips: &[
            IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)),
            IpAddr::V4(Ipv4Addr::new(149, 112, 112, 112)),
        ],
    },
    Upstream {
        flag_leaf: "opendns",
        label: "OpenDNS",
        ips: &[
            IpAddr::V4(Ipv4Addr::new(208, 67, 222, 222)),
            IpAddr::V4(Ipv4Addr::new(208, 67, 220, 220)),
        ],
    },
    Upstream {
        flag_leaf: "adguard",
        label: "AdGuard",
        ips: &[
            IpAddr::V4(Ipv4Addr::new(94, 140, 14, 14)),
            IpAddr::V4(Ipv4Addr::new(94, 140, 15, 15)),
        ],
    },
    Upstream {
        flag_leaf: "yandex",
        label: "Yandex",
        ips: &[
            IpAddr::V4(Ipv4Addr::new(77, 88, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(77, 88, 8, 1)),
        ],
    },
];

fn lookup_upstream(name: &str) -> Option<&'static Upstream> {
    UPSTREAMS
        .iter()
        .find(|u| u.flag_leaf.eq_ignore_ascii_case(name) || u.label.eq_ignore_ascii_case(name))
}

/// Resolvers selected for fan-out queries: every `dns.resolvers.*` leaf
/// that resolves to `on`.
fn enabled_upstreams(rules: &[(String, String)]) -> Vec<&'static Upstream> {
    UPSTREAMS
        .iter()
        .filter(|u| {
            let flag = format!("dns.resolvers.{}", u.flag_leaf);
            features::parse_bool(&features::resolve(rules, &flag).0).unwrap_or(true)
        })
        .collect()
}

/// Per-chat timeout budget for the parallel fan-out, in milliseconds.
fn parallel_timeout(rules: &[(String, String)]) -> Duration {
    let v = features::resolve(rules, "dns.parallel_timeout").0;
    let ms: u64 = v.parse().unwrap_or(3000);
    Duration::from_millis(ms)
}

/// Build a resolver pointed at one upstream IP. Validation toggles the
/// DNSSEC machinery; we leave it off for plain lookups and turn it on
/// for `/dns sec`.
fn build_resolver_for_ip(ip: IpAddr, validate: bool) -> Result<TokioResolver, String> {
    let config = ResolverConfig::from_parts(
        None,
        Vec::new(),
        vec![NameServerConfig::udp_and_tcp(ip)],
    );
    let mut builder = Resolver::builder_with_config(config, TokioRuntimeProvider::default());
    {
        let opts = builder.options_mut();
        opts.attempts = 2;
        opts.timeout = Duration::from_secs(2);
        opts.validate = validate;
        opts.use_hosts_file = hickory_resolver::config::ResolveHosts::Never;
    }
    builder.build().map_err(|e| format!("build resolver: {e}"))
}

/// One row of a lookup result: the record-type + an opaque rendered RData
/// + TTL.
#[derive(Debug, Clone)]
pub struct Row {
    pub record_type: RecordType,
    pub value: String,
    pub ttl: u32,
}

impl Row {
    fn from_record(rec: &Record) -> Option<Self> {
        Some(Self {
            record_type: rec.record_type(),
            value: rdata_to_string(&rec.data),
            ttl: rec.ttl,
        })
    }
}

/// Render a hickory `RData` value for human consumption. Closer to
/// `dig`'s presentation than Rust's default `Debug`.
fn rdata_to_string(data: &RData) -> String {
    match data {
        RData::A(a) => a.0.to_string(),
        RData::AAAA(a) => a.0.to_string(),
        RData::CNAME(c) => c.0.to_string(),
        RData::NS(n) => n.0.to_string(),
        RData::PTR(p) => p.0.to_string(),
        RData::MX(m) => format!("{} {}", m.preference, m.exchange),
        RData::TXT(t) => t
            .txt_data
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
            .collect::<Vec<_>>()
            .join(""),
        RData::SOA(s) => format!(
            "{} {} {} {} {} {} {}",
            s.mname, s.rname, s.serial, s.refresh, s.retry, s.expire, s.minimum
        ),
        RData::SRV(s) => format!("{} {} {} {}", s.priority, s.weight, s.port, s.target),
        RData::CAA(c) => {
            // CAA value field is bytes — most commonly UTF-8 text (e.g. an
            // issuer hostname or a mailto: URI). Render leniently.
            let val = String::from_utf8_lossy(&c.value);
            format!("{} {} \"{}\"", c.issuer_critical as u8, c.tag, val)
        }
        RData::DNSSEC(d) => format_dnssec_rdata(d),
        other => format!("{other:?}"),
    }
}

/// Best-effort short rendering for DNSSEC-family records. The full
/// signature/key blobs are noise inside a chat reply; show the
/// structural fields and a truncated fingerprint instead of dumping
/// hundreds of bytes through `Debug`.
fn format_dnssec_rdata(d: &hickory_proto::dnssec::rdata::DNSSECRData) -> String {
    use hickory_proto::dnssec::rdata::DNSSECRData as D;
    match d {
        D::RRSIG(s) => {
            // RRSIG derefs to SIG, whose `input()` returns a SigInput.
            let i = s.input();
            format!(
                "RRSIG {} alg={:?} labels={} ttl={} kt={} signer={}",
                i.type_covered,
                i.algorithm,
                i.num_labels,
                i.original_ttl,
                i.key_tag,
                i.signer_name
            )
        }
        D::DNSKEY(k) => format!(
            "DNSKEY flags={} kt={}",
            k.flags(),
            k.calculate_key_tag().unwrap_or(0)
        ),
        D::DS(ds) => format!(
            "DS kt={} alg={:?} digest={:?}",
            ds.key_tag(),
            ds.algorithm(),
            ds.digest_type()
        ),
        other => format!("{other:?}"),
    }
}

/// Outcome of a single resolver query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub rows: Vec<Row>,
    pub rtt: Duration,
    pub error: Option<String>,
}

impl QueryResult {
    fn err(rtt: Duration, msg: impl Into<String>) -> Self {
        Self {
            rows: Vec::new(),
            rtt,
            error: Some(msg.into()),
        }
    }
}

/// Run one query against one upstream IP. Captures RTT.
pub async fn query_at(
    upstream_ip: IpAddr,
    name: &str,
    rtype: RecordType,
    timeout: Duration,
) -> QueryResult {
    let resolver = match build_resolver_for_ip(upstream_ip, false) {
        Ok(r) => Arc::new(r),
        Err(e) => return QueryResult::err(Duration::ZERO, e),
    };
    let parsed = match Name::from_utf8(name) {
        Ok(n) => n,
        Err(e) => return QueryResult::err(Duration::ZERO, format!("bad name: {e}")),
    };
    let start = Instant::now();
    let fut = resolver.lookup(parsed, rtype);
    let res = match tokio::time::timeout(timeout, fut).await {
        Ok(r) => r,
        Err(_) => return QueryResult::err(timeout, format!("timeout >{}ms", timeout.as_millis())),
    };
    let rtt = start.elapsed();
    match res {
        Ok(lookup) => {
            let rows: Vec<Row> = lookup.answers().iter().filter_map(Row::from_record).collect();
            QueryResult {
                rows,
                rtt,
                error: None,
            }
        }
        Err(e) => {
            // NXDOMAIN / NoRecords are common and worth showing distinctly.
            let msg = if e.is_nx_domain() {
                "NXDOMAIN".to_string()
            } else if e.is_no_records_found() {
                "no records".to_string()
            } else {
                e.to_string()
            };
            QueryResult::err(rtt, msg)
        }
    }
}

/// Parse a resolver token from the user command: an IP literal, a known
/// resolver label/flag-leaf, or a hostname (which we resolve via the
/// system pool first). Returns the IP we should send the query to.
pub async fn resolve_upstream_arg(arg: &str) -> Result<IpAddr, String> {
    let arg = arg.trim_start_matches('@');
    if let Ok(ip) = arg.parse::<IpAddr>() {
        return Ok(ip);
    }
    if let Some(u) = lookup_upstream(arg) {
        return Ok(u.ips[0]);
    }
    // Last resort: ask one of the default resolvers (Cloudflare) for an A.
    let res = query_at(
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        arg,
        RecordType::A,
        Duration::from_secs(2),
    )
    .await;
    res.rows
        .into_iter()
        .find_map(|r| r.value.parse::<Ipv4Addr>().ok().map(IpAddr::V4))
        .or_else(|| {
            // try v6 fallback
            None::<IpAddr>
        })
        .ok_or_else(|| format!("не нашёл резолвер {arg:?}"))
}

/// Translate a user-supplied type token into a `RecordType`. Defaults to
/// `A` when empty; understands the popular extras.
pub fn parse_record_type(tok: &str) -> Result<RecordType, String> {
    let t = tok.to_ascii_uppercase();
    let rt = match t.as_str() {
        "" | "A" => RecordType::A,
        "AAAA" => RecordType::AAAA,
        "ANY" => RecordType::ANY,
        "MX" => RecordType::MX,
        "NS" => RecordType::NS,
        "TXT" => RecordType::TXT,
        "SOA" => RecordType::SOA,
        "CNAME" => RecordType::CNAME,
        "PTR" => RecordType::PTR,
        "CAA" => RecordType::CAA,
        "SRV" => RecordType::SRV,
        "DS" => RecordType::DS,
        "DNSKEY" => RecordType::DNSKEY,
        "TLSA" => RecordType::TLSA,
        "HTTPS" => RecordType::HTTPS,
        "SVCB" => RecordType::SVCB,
        "NAPTR" => RecordType::NAPTR,
        "SSHFP" => RecordType::SSHFP,
        "RRSIG" => RecordType::RRSIG,
        other => return Err(format!("неизвестный тип записи: {other}")),
    };
    Ok(rt)
}

// ---------- subcommand entrypoints ----------

/// Dispatch `/dns <rest>` to the subcommand handler. Returns the body
/// text to send back; caller wraps in reply parameters.
pub async fn handle(rest: &str, rules: &[(String, String)]) -> String {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.is_empty() {
        return help_text().to_string();
    }
    match tokens[0].to_ascii_lowercase().as_str() {
        "help" | "?" => help_text().to_string(),
        "prop" => cmd_prop(&tokens[1..], rules).await,
        "sec" => cmd_sec(&tokens[1..], rules).await,
        "spf" => cmd_spf(&tokens[1..], rules).await,
        "dmarc" => cmd_dmarc(&tokens[1..], rules).await,
        "mx" => cmd_mx(&tokens[1..], rules).await,
        "mta" => cmd_mta(&tokens[1..], rules).await,
        "caa" => cmd_caa(&tokens[1..], rules).await,
        _ => cmd_lookup(&tokens, rules).await,
    }
}

fn help_text() -> &'static str {
    "/dns — DNS-тулчейн.\n\
     Подкоманды:\n\
     * /dns <name> [type] [@resolver] — обычный lookup\n\
     * /dns prop <name> [type] — таблица пропагации\n\
     * /dns sec <name> — валидация DNSSEC-цепочки\n\
     * /dns spf <domain> — раскрыть SPF, посчитать lookups\n\
     * /dns dmarc <domain> — разобрать DMARC-политику\n\
     * /dns mx <domain> — MX + PTR + STARTTLS + DANE\n\
     * /dns mta <domain> — MTA-STS политика\n\
     * /dns caa <name> — обход CAA вверх по зоне"
}

// ---------- /dns <name> [type] [@resolver] ----------

async fn cmd_lookup(tokens: &[&str], rules: &[(String, String)]) -> String {
    // Split tokens: [<name>] [<type>?] [@<resolver>?]
    let mut name: Option<&str> = None;
    let mut rtype_tok: &str = "";
    let mut resolver_tok: Option<&str> = None;
    for t in tokens {
        if let Some(stripped) = t.strip_prefix('@') {
            resolver_tok = Some(stripped);
        } else if name.is_none() {
            name = Some(*t);
        } else {
            rtype_tok = *t;
        }
    }
    let Some(name) = name else {
        return "формат: /dns <name> [type] [@resolver]".to_string();
    };
    let rtype = match parse_record_type(rtype_tok) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let timeout = parallel_timeout(rules);

    let upstream: (String, IpAddr) = match resolver_tok {
        Some(arg) => match resolve_upstream_arg(arg).await {
            Ok(ip) => (arg.to_string(), ip),
            Err(e) => return e,
        },
        None => {
            // Pick the first enabled resolver from the chat's set.
            let enabled = enabled_upstreams(rules);
            let u = enabled.first().copied().unwrap_or(&UPSTREAMS[0]);
            (u.label.to_string(), u.ips[0])
        }
    };

    let q = query_at(upstream.1, name, rtype, timeout).await;
    format_single_lookup(&upstream.0, upstream.1, name, rtype, &q, rules)
}

fn format_single_lookup(
    upstream_label: &str,
    upstream_ip: IpAddr,
    name: &str,
    rtype: RecordType,
    q: &QueryResult,
    rules: &[(String, String)],
) -> String {
    let show_ttl = features::parse_bool(&features::resolve(rules, "dns.show_ttl").0)
        .unwrap_or(true);
    let show_rtt = features::parse_bool(&features::resolve(rules, "dns.show_rtt").0)
        .unwrap_or(true);
    let mut out = format!(
        "; {} {} @{} ({})\n",
        name, rtype, upstream_label, upstream_ip
    );
    if let Some(err) = &q.error {
        out.push_str(&format!("; ошибка: {err}\n"));
    }
    for row in &q.rows {
        if show_ttl {
            out.push_str(&format!(
                "{:<28} {:>6} {:<6} {}\n",
                trim_name(name),
                row.ttl,
                row.record_type,
                row.value
            ));
        } else {
            out.push_str(&format!(
                "{:<28} {:<6} {}\n",
                trim_name(name),
                row.record_type,
                row.value
            ));
        }
    }
    if show_rtt {
        out.push_str(&format!("; rtt {} ms\n", q.rtt.as_millis()));
    }
    out
}

fn trim_name(s: &str) -> &str {
    if s.len() > 28 {
        &s[..28]
    } else {
        s
    }
}

// ---------- /dns prop <name> [type] ----------

async fn cmd_prop(tokens: &[&str], rules: &[(String, String)]) -> String {
    let name = match tokens.first() {
        Some(n) => *n,
        None => return "формат: /dns prop <name> [type]".to_string(),
    };
    let rtype_tok = tokens.get(1).copied().unwrap_or("");
    let rtype = match parse_record_type(rtype_tok) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let timeout = parallel_timeout(rules);
    let upstreams = enabled_upstreams(rules);
    if upstreams.is_empty() {
        return "не ква, ни одного резолвера не выбрано — /feature dns.resolvers".to_string();
    }

    // Fan out across resolvers in parallel.
    let queries = upstreams.iter().map(|u| {
        let ip = u.ips[0];
        let name = name.to_string();
        async move {
            let q = query_at(ip, &name, rtype, timeout).await;
            (u.label, ip, q)
        }
    });
    let results = futures::future::join_all(queries).await;

    // Group resolvers by the normalized answer set. We need the order of
    // appearance later for stable cohort assignment, so use Vec instead of
    // BTreeMap.
    let mut cohort_keys: Vec<String> = Vec::new();
    let mut cohort_members: Vec<Vec<usize>> = Vec::new(); // indices into `results`
    for (i, (_label, _ip, q)) in results.iter().enumerate() {
        let mut vals: Vec<String> = q.rows.iter().map(|r| r.value.clone()).collect();
        vals.sort();
        let key = if let Some(err) = &q.error {
            format!("!{err}")
        } else if vals.is_empty() {
            "(empty)".to_string()
        } else {
            vals.join(",")
        };
        match cohort_keys.iter().position(|k| k == &key) {
            Some(idx) => cohort_members[idx].push(i),
            None => {
                cohort_keys.push(key);
                cohort_members.push(vec![i]);
            }
        }
    }

    // Reorder cohorts so the largest comes first (= "A"). Ties broken by
    // first appearance to keep results stable across runs.
    let mut order: Vec<usize> = (0..cohort_keys.len()).collect();
    order.sort_by(|a, b| cohort_members[*b].len().cmp(&cohort_members[*a].len()));
    // Build resolver_index → cohort_letter
    let mut resolver_letter: Vec<char> = vec![' '; results.len()];
    for (rank, cohort_idx) in order.iter().enumerate() {
        let letter = char::from(b'A' + rank as u8);
        for &res_idx in &cohort_members[*cohort_idx] {
            resolver_letter[res_idx] = letter;
        }
    }

    let unanimous = cohort_keys.len() == 1;
    let show_rtt = features::parse_bool(&features::resolve(rules, "dns.show_rtt").0)
        .unwrap_or(true);

    let mut out = format!("; {} {} — пропагация\n", name, rtype);
    if unanimous {
        out.push_str("; все резолверы согласны ✅\n");
    } else {
        // Header lines summarize cohort sizes, e.g. "A=3 B=2 C=1".
        let summary: Vec<String> = order
            .iter()
            .enumerate()
            .map(|(rank, cohort_idx)| {
                let letter = char::from(b'A' + rank as u8);
                format!("{}={}", letter, cohort_members[*cohort_idx].len())
            })
            .collect();
        out.push_str(&format!(
            "; {} групп ответов: {} ⚠️\n",
            cohort_keys.len(),
            summary.join(" ")
        ));
    }

    for (i, (label, ip, q)) in results.iter().enumerate() {
        let letter = resolver_letter[i];
        let badge = if q.error.is_some() {
            "❌".to_string()
        } else if unanimous {
            "✅".to_string()
        } else if letter == 'A' {
            // Majority cohort — green tick to anchor the eye.
            format!("[{letter}] ✅")
        } else {
            format!("[{letter}] ⚠️")
        };
        let rtt_str = if show_rtt {
            format!("{:>4}ms", q.rtt.as_millis())
        } else {
            String::new()
        };
        out.push_str(&format!("{badge} {:<11} {:<15} {}", label, ip, rtt_str));
        if let Some(err) = &q.error {
            out.push_str(&format!("   {err}\n"));
            continue;
        }
        if q.rows.is_empty() {
            out.push_str("   (пусто)\n");
            continue;
        }
        out.push('\n');
        for row in &q.rows {
            out.push_str(&format!("   {} {}\n", row.record_type, row.value));
        }
    }
    out
}

// ---------- /dns sec <name> ----------

async fn cmd_sec(tokens: &[&str], rules: &[(String, String)]) -> String {
    let name = match tokens.first() {
        Some(n) => *n,
        None => return "формат: /dns sec <name>".to_string(),
    };
    let timeout = parallel_timeout(rules);
    let pick = enabled_upstreams(rules)
        .first()
        .copied()
        .unwrap_or(&UPSTREAMS[0]);
    let upstream_ip = pick.ips[0];

    // Three independent queries, all in parallel: the validating lookup
    // (tells us "is this name's chain valid OR is the zone insecure"),
    // and the DS/DNSKEY at the zone apex (tells us "is the zone signed
    // at all"). The combination distinguishes secure / insecure / bogus.
    let zone = zone_apex(name);
    let validating = match build_resolver_for_ip(upstream_ip, true) {
        Ok(r) => Arc::new(r),
        Err(e) => return format!("не ква: {e}"),
    };
    let parsed = match Name::from_utf8(name) {
        Ok(n) => n,
        Err(e) => return format!("bad name: {e}"),
    };
    let start = Instant::now();
    let lookup_fut = tokio::time::timeout(timeout, validating.lookup(parsed, RecordType::A));
    let ds_fut = query_at(upstream_ip, &zone, RecordType::DS, timeout);
    let dnskey_fut = query_at(upstream_ip, &zone, RecordType::DNSKEY, timeout);
    let (lookup_res, ds_q, dnskey_q) = futures::join!(lookup_fut, ds_fut, dnskey_fut);
    let rtt = start.elapsed();

    let signed_at_parent = ds_q.error.is_none() && !ds_q.rows.is_empty();
    let signed_at_zone = dnskey_q.error.is_none() && !dnskey_q.rows.is_empty();
    let zone_signed = signed_at_parent && signed_at_zone;

    let mut out = format!("; {} — DNSSEC через {}\n", name, pick.label);
    match lookup_res {
        Err(_) => out.push_str(&format!("⌛ таймаут {} ms\n", rtt.as_millis())),
        Ok(Err(e)) if zone_signed => {
            // The zone IS signed, and the validating resolver bailed.
            // That's a real DNSSEC failure.
            out.push_str(&format!("❌ цепочка сломана (bogus): {e}\n"));
        }
        Ok(Err(e)) => {
            // Zone isn't signed and the lookup failed for some other
            // reason — propagate the error verbatim.
            out.push_str(&format!("⚠️ {e}\n"));
        }
        Ok(Ok(lookup)) => {
            if zone_signed {
                out.push_str("✅ цепочка валидна (signed)\n");
            } else {
                out.push_str("ℹ️ зона не подписана DNSSEC (insecure)\n");
            }
            for rec in lookup.answers() {
                // RRSIG records would just repeat the signature blob we
                // already validated; filter them out of the user view.
                if rec.record_type() == RecordType::RRSIG {
                    continue;
                }
                out.push_str(&format!(
                    "   {} {} {}\n",
                    rec.record_type(),
                    rec.ttl,
                    rdata_to_string(&rec.data)
                ));
            }
        }
    }

    out.push_str("\n; цепочка на апексе зоны:\n");
    let ds_badge = if ds_q.error.is_some() {
        "❌"
    } else if ds_q.rows.is_empty() {
        "·"
    } else {
        "✅"
    };
    out.push_str(&format!(
        "   {ds_badge} {zone} DS    @parent: {}\n",
        if let Some(err) = &ds_q.error {
            err.clone()
        } else if ds_q.rows.is_empty() {
            "нет (зона не подписана)".to_string()
        } else {
            format!("{} записей", ds_q.rows.len())
        }
    ));
    let dnskey_badge = if dnskey_q.error.is_some() {
        "❌"
    } else if dnskey_q.rows.is_empty() {
        "·"
    } else {
        "✅"
    };
    out.push_str(&format!(
        "   {dnskey_badge} {zone} DNSKEY:        {}\n",
        if let Some(err) = &dnskey_q.error {
            err.clone()
        } else if dnskey_q.rows.is_empty() {
            "нет".to_string()
        } else {
            format!("{} записей", dnskey_q.rows.len())
        }
    ));
    out
}

/// Best-effort zone apex guess: drop the leftmost label until a single
/// label is left, then return that joined to the rest. For most public
/// zones this matches the apex; for ccSLDs (`co.uk`) it'll be wrong, but
/// that's a depth-2 corner case the user can work around.
fn zone_apex(name: &str) -> String {
    let n = name.trim_end_matches('.');
    let labels: Vec<&str> = n.split('.').collect();
    if labels.len() <= 2 {
        return n.to_string();
    }
    labels[labels.len() - 2..].join(".")
}

// ---------- /dns spf, dmarc, mx, mta, caa — Tier 2 ----------

async fn cmd_spf(tokens: &[&str], rules: &[(String, String)]) -> String {
    let domain = match tokens.first() {
        Some(d) => *d,
        None => return "формат: /dns spf <domain>".to_string(),
    };
    let timeout = parallel_timeout(rules);
    let pick = enabled_upstreams(rules)
        .first()
        .copied()
        .unwrap_or(&UPSTREAMS[0]);
    let mut visited: Vec<String> = Vec::new();
    let mut total_lookups = 0usize;
    let mut tree: Vec<String> = Vec::new();
    spf_expand(
        domain,
        pick.ips[0],
        timeout,
        &mut visited,
        &mut total_lookups,
        &mut tree,
        0,
    )
    .await;

    let mut out = format!("; SPF {}\n", domain);
    if tree.is_empty() {
        out.push_str("; не нашёл v=spf1 TXT\n");
        return out;
    }
    out.push_str(&tree.join("\n"));
    out.push('\n');
    let cap = 10;
    let mark = if total_lookups <= cap { "✅" } else { "❌" };
    out.push_str(&format!(
        "; lookups: {}/{} {} (RFC 7208 §4.6.4)\n",
        total_lookups, cap, mark
    ));
    out
}

fn spf_expand<'a>(
    domain: &'a str,
    upstream: IpAddr,
    timeout: Duration,
    visited: &'a mut Vec<String>,
    lookups: &'a mut usize,
    tree: &'a mut Vec<String>,
    depth: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let lower = domain.to_ascii_lowercase();
        if visited.contains(&lower) {
            tree.push(format!("{}↳ {} (повтор — пропуск)", indent(depth), domain));
            return;
        }
        visited.push(lower);

        let q = query_at(upstream, domain, RecordType::TXT, timeout).await;
        let spf_txt = q
            .rows
            .iter()
            .map(|r| r.value.clone())
            .find(|v| v.to_ascii_lowercase().starts_with("v=spf1"));
        let Some(spf) = spf_txt else {
            tree.push(format!("{}↳ {} — нет v=spf1", indent(depth), domain));
            return;
        };
        tree.push(format!("{}↳ {} — {}", indent(depth), domain, spf));

        for term in spf.split_whitespace().skip(1) {
            let t = term.trim_start_matches(['+', '-', '~', '?']);
            if let Some(rest) = t.strip_prefix("include:") {
                *lookups += 1;
                spf_expand(rest, upstream, timeout, visited, lookups, tree, depth + 1).await;
            } else if let Some(rest) = t.strip_prefix("redirect=") {
                *lookups += 1;
                spf_expand(rest, upstream, timeout, visited, lookups, tree, depth + 1).await;
            } else if t == "a" || t.starts_with("a:") || t == "mx" || t.starts_with("mx:") {
                *lookups += 1;
            } else if t.starts_with("exists:") || t.starts_with("ptr") {
                *lookups += 1;
            }
        }
    })
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

async fn cmd_dmarc(tokens: &[&str], rules: &[(String, String)]) -> String {
    let domain = match tokens.first() {
        Some(d) => *d,
        None => return "формат: /dns dmarc <domain>".to_string(),
    };
    let timeout = parallel_timeout(rules);
    let pick = enabled_upstreams(rules)
        .first()
        .copied()
        .unwrap_or(&UPSTREAMS[0]);
    let name = format!("_dmarc.{}", domain);
    let q = query_at(pick.ips[0], &name, RecordType::TXT, timeout).await;
    let Some(dmarc) = q
        .rows
        .into_iter()
        .map(|r| r.value)
        .find(|v| v.to_ascii_lowercase().starts_with("v=dmarc1"))
    else {
        return format!("; {} — DMARC не найден (или не TXT)\n", name);
    };
    let mut out = format!("; DMARC {}\n{}\n\n", domain, dmarc);
    for part in dmarc.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (tag, val) = part.split_once('=').unwrap_or((part, ""));
        let explanation = explain_dmarc_tag(tag.trim(), val.trim());
        out.push_str(&format!("  {tag}={val}  — {explanation}\n"));
    }
    out
}

fn explain_dmarc_tag(tag: &str, val: &str) -> &'static str {
    match tag {
        "v" => "версия (всегда DMARC1)",
        "p" => match val {
            "none" => "только мониторинг — почта не блокируется при провале",
            "quarantine" => "при провале — в спам",
            "reject" => "при провале — отказ принимать письмо",
            _ => "политика для домена",
        },
        "sp" => "политика для поддоменов (наследует p, если опущено)",
        "pct" => "процент писем, к которым применяется политика (100 = ко всем)",
        "rua" => "куда слать агрегированные отчёты (URI mailto:)",
        "ruf" => "куда слать forensic-отчёты по отдельным провалам",
        "adkim" => "выравнивание DKIM: s=strict, r=relaxed",
        "aspf" => "выравнивание SPF: s=strict, r=relaxed",
        "fo" => "когда слать forensic: 0/1/d/s — комбинация",
        "rf" => "формат forensic-отчёта (afrf по умолчанию)",
        "ri" => "интервал агрегированных отчётов в секундах",
        _ => "(нестандартный тег)",
    }
}

async fn cmd_mx(tokens: &[&str], rules: &[(String, String)]) -> String {
    let domain = match tokens.first() {
        Some(d) => *d,
        None => return "формат: /dns mx <domain>".to_string(),
    };
    let timeout = parallel_timeout(rules);
    let pick = enabled_upstreams(rules)
        .first()
        .copied()
        .unwrap_or(&UPSTREAMS[0]);
    let upstream_ip = pick.ips[0];
    let q = query_at(upstream_ip, domain, RecordType::MX, timeout).await;
    if q.rows.is_empty() {
        return format!(
            "; {} — нет MX{}\n",
            domain,
            q.error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        );
    }
    let mut mxes: Vec<(u16, String)> = q
        .rows
        .iter()
        .filter_map(|r| {
            let mut parts = r.value.splitn(2, ' ');
            let pref = parts.next()?.parse::<u16>().ok()?;
            let host = parts.next()?.trim_end_matches('.').to_string();
            Some((pref, host))
        })
        .collect();
    mxes.sort_by_key(|(p, _)| *p);

    // Probe each MX host in parallel. The slow leg is the TLS handshake;
    // serializing 5 MX × ~1–3s adds up fast.
    let probes = mxes.iter().map(|(pref, host)| {
        let host = host.clone();
        let pref = *pref;
        async move {
            let block = probe_mx_host(upstream_ip, &host, timeout).await;
            (pref, host, block)
        }
    });
    let results = futures::future::join_all(probes).await;

    let mut out = format!("; MX {} ({} записей)\n", domain, mxes.len());
    for (pref, host, block) in results {
        out.push_str(&format!("\n[{}] {}\n", pref, host));
        out.push_str(&block);
    }
    out
}

/// Per-MX probe: parallel A + TLSA lookups, then a PTR per resolved
/// address and a STARTTLS handshake. Returns the formatted block for
/// that host (no header line — caller adds `[pref] host`).
async fn probe_mx_host(upstream_ip: IpAddr, host: &str, timeout: Duration) -> String {
    let a_fut = query_at(upstream_ip, host, RecordType::A, timeout);
    let tlsa_name = format!("_25._tcp.{}", host);
    let tlsa_fut = query_at(upstream_ip, &tlsa_name, RecordType::TLSA, timeout);
    let tls_fut = probe_starttls(host, timeout);
    let (a, tlsa, tls) = futures::join!(a_fut, tlsa_fut, tls_fut);

    let mut out = String::new();
    // A records (+ PTR per address, sequential per address since PTR
    // depends on the resolved A and the addresses are usually 1).
    for r in &a.rows {
        out.push_str(&format!("   A     {} (ttl {})\n", r.value, r.ttl));
        let ptr_name = ptr_name_for(&r.value);
        let ptr = query_at(upstream_ip, &ptr_name, RecordType::PTR, timeout).await;
        if let Some(p) = ptr.rows.first() {
            out.push_str(&format!("   PTR   {}\n", p.value));
        }
    }
    match tls {
        Ok(info) => out.push_str(&format!("   TLS   {}\n", info)),
        Err(e) => out.push_str(&format!("   TLS   ❌ {}\n", e)),
    }
    if !tlsa.rows.is_empty() {
        out.push_str(&format!("   DANE  ✅ {} TLSA записей\n", tlsa.rows.len()));
    }
    out
}

fn ptr_name_for(ip: &str) -> String {
    if let Ok(v4) = ip.parse::<Ipv4Addr>() {
        let o = v4.octets();
        return format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0]);
    }
    if let Ok(v6) = ip.parse::<Ipv6Addr>() {
        let mut out = String::new();
        for byte in v6.octets().iter().rev() {
            out.push_str(&format!("{:x}.{:x}.", byte & 0xF, byte >> 4));
        }
        out.push_str("ip6.arpa");
        return out;
    }
    ip.to_string()
}

/// SMTP STARTTLS probe: TCP-connect to port 25, banner, EHLO, STARTTLS,
/// TLS handshake, pull the leaf cert. Returns one-line summary.
///
/// Single TCP connect — we do the SMTP dance on the split halves, then
/// reunite for the TLS upgrade. The 220-STARTTLS reply is one line and
/// the server is required to stay silent until the TLS handshake begins,
/// so `BufReader::into_inner` discarding its buffer is safe here.
async fn probe_starttls(host: &str, timeout: Duration) -> Result<String, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    let target = format!("{host}:25");
    let stream = tokio::time::timeout(timeout, TcpStream::connect(&target))
        .await
        .map_err(|_| "TCP timeout".to_string())?
        .map_err(|e| classify_tcp_error(&e))?;
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);

    // Banner: lines until one starts with "220 " (terminal greeting).
    loop {
        let mut l = String::new();
        let n = rd.read_line(&mut l).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("сервер закрыл коннект до баннера".to_string());
        }
        if !l.starts_with("220") {
            return Err(format!("банер не 220: {}", l.trim()));
        }
        if l.starts_with("220 ") {
            break;
        }
    }
    wr.write_all(b"EHLO mallard-bot\r\n")
        .await
        .map_err(|e| e.to_string())?;
    // Drain EHLO response — lines starting with "250-" continue, "250 " ends.
    loop {
        let mut l = String::new();
        let n = rd.read_line(&mut l).await.map_err(|e| e.to_string())?;
        if n == 0 || l.starts_with("250 ") {
            break;
        }
    }
    wr.write_all(b"STARTTLS\r\n")
        .await
        .map_err(|e| e.to_string())?;
    let mut r = String::new();
    rd.read_line(&mut r).await.map_err(|e| e.to_string())?;
    if !r.starts_with("220") {
        return Err(format!("STARTTLS отказ: {}", r.trim()));
    }

    // Reunite halves into a single TcpStream for the TLS handshake.
    let socket: TcpStream = rd.into_inner().reunite(wr).map_err(|e| e.to_string())?;

    let mut root = rustls::RootCertStore::empty();
    root.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name = rustls_pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| format!("server name: {e}"))?;

    let tls = tokio::time::timeout(timeout, connector.connect(server_name, socket))
        .await
        .map_err(|_| "TLS timeout".to_string())?
        .map_err(|e| format!("TLS handshake: {e}"))?;
    let (_io, conn) = tls.into_inner();
    let certs = conn
        .peer_certificates()
        .ok_or_else(|| "нет сертификатов".to_string())?;
    let leaf = certs.first().ok_or_else(|| "нет leaf".to_string())?;
    let parsed = x509_parser::parse_x509_certificate(leaf.as_ref())
        .map_err(|e| format!("parse cert: {e}"))?
        .1;
    let subject = parsed.subject().to_string();
    let issuer = parsed.issuer().to_string();
    let not_after = parsed.validity().not_after.to_string();
    let proto = conn
        .protocol_version()
        .map(|v| format!("{v:?}"))
        .unwrap_or_default();
    Ok(format!(
        "✅ {proto} | subject={subject} | issuer={issuer} | до {not_after}"
    ))
}

/// Translate a TCP-connect `io::Error` into a short status string. Stay
/// neutral on root cause — ENETUNREACH can be the destination not running
/// SMTP, the bot's egress being filtered, or routing in between. Leave
/// the diagnosis to the human.
fn classify_tcp_error(e: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::ConnectionRefused => "TCP refused (порт 25 закрыт)".to_string(),
        ErrorKind::TimedOut => "TCP timeout".to_string(),
        _ if e.raw_os_error() == Some(101) => "TCP: network unreachable".to_string(),
        _ if e.raw_os_error() == Some(113) => "TCP: no route to host".to_string(),
        _ => format!("TCP: {e}"),
    }
}

async fn cmd_mta(tokens: &[&str], rules: &[(String, String)]) -> String {
    let domain = match tokens.first() {
        Some(d) => *d,
        None => return "формат: /dns mta <domain>".to_string(),
    };
    let timeout = parallel_timeout(rules);
    let pick = enabled_upstreams(rules)
        .first()
        .copied()
        .unwrap_or(&UPSTREAMS[0]);

    // _mta-sts.<domain> TXT first — gives the policy ID.
    let id_q = query_at(
        pick.ips[0],
        &format!("_mta-sts.{}", domain),
        RecordType::TXT,
        timeout,
    )
    .await;
    let mut out = format!("; MTA-STS {}\n", domain);
    match id_q.rows.first() {
        Some(r) => out.push_str(&format!("; _mta-sts TXT: {}\n", r.value)),
        None => out.push_str("; _mta-sts TXT: нет — MTA-STS, скорее всего, не включен\n"),
    }

    // Fetch the policy file over HTTPS.
    let url = format!("https://mta-sts.{}/.well-known/mta-sts.txt", domain);
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("mallard-bot")
        .build()
        .map_err(|e| e.to_string());
    match client {
        Err(e) => out.push_str(&format!("❌ http client: {e}\n")),
        Ok(c) => match c.get(&url).send().await {
            Err(e) => out.push_str(&format!("❌ fetch {url}: {e}\n")),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    out.push_str(&format!("❌ {status}\n"));
                } else {
                    out.push_str(&format!("✅ {status} — {} байт\n\n", body.len()));
                    out.push_str(body.trim());
                    out.push('\n');
                }
            }
        },
    }

    // TLS-RPT
    let rpt_q = query_at(
        pick.ips[0],
        &format!("_smtp._tls.{}", domain),
        RecordType::TXT,
        timeout,
    )
    .await;
    out.push_str("\n; TLS-RPT\n");
    match rpt_q.rows.first() {
        Some(r) => out.push_str(&format!("  {}\n", r.value)),
        None => out.push_str("  нет _smtp._tls TXT\n"),
    }
    out
}

async fn cmd_caa(tokens: &[&str], rules: &[(String, String)]) -> String {
    let name = match tokens.first() {
        Some(n) => *n,
        None => return "формат: /dns caa <name>".to_string(),
    };
    let timeout = parallel_timeout(rules);
    let pick = enabled_upstreams(rules)
        .first()
        .copied()
        .unwrap_or(&UPSTREAMS[0]);
    let mut out = format!("; CAA-обход {}\n", name);
    let labels: Vec<&str> = name.trim_end_matches('.').split('.').collect();
    let mut found = false;
    for i in 0..labels.len() {
        let probe = labels[i..].join(".");
        let q = query_at(pick.ips[0], &probe, RecordType::CAA, timeout).await;
        if !q.rows.is_empty() {
            out.push_str(&format!("✅ {} — {} записей\n", probe, q.rows.len()));
            for r in &q.rows {
                out.push_str(&format!("   {}\n", r.value));
            }
            found = true;
            break;
        } else {
            out.push_str(&format!("·  {} — нет CAA\n", probe));
        }
    }
    if !found {
        out.push_str("; CAA не найден до апекса — любой CA может выпускать сертификаты\n");
    }
    out
}
