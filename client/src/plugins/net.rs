use super::manifest;
use serde_json::{Map, Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const RESPONSE_LIMIT: usize = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (18..=19).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80)
        }
    }
}

async fn resolve(host: &str) -> Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 443)).await.map_err(|_| format!("couldn't find {host}"))?.collect();
    if addrs.is_empty() {
        return Err(format!("couldn't find {host}"));
    }
    if addrs.iter().any(|a| !public_ip(a.ip())) {
        return Err(format!("{host} points to a private network address, which plugins can't reach"));
    }
    Ok(addrs)
}

pub async fn fetch(url: String, method: String, headers: Vec<(String, String)>, body: Option<String>, hosts: Vec<String>) -> Result<String, String> {
    let first_host = reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    let mut url = url;
    let mut method = method;
    let mut body = body;
    for _ in 0..=MAX_REDIRECTS {
        if !manifest::host_allowed(&hosts, &url) {
            return Err(format!("this plugin can only connect to {} over https", hosts.join(", ")));
        }
        let parsed = reqwest::Url::parse(&url).map_err(|_| "invalid address".to_string())?;
        let host = parsed.host_str().unwrap_or_default().to_owned();
        let addrs = resolve(&host).await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&host, &addrs)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("Rustify plugin")
            .build()
            .map_err(|e| e.to_string())?;
        let verb = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "invalid method".to_string())?;
        let mut request = client.request(verb, parsed.clone());
        let same_host = host.eq_ignore_ascii_case(&first_host);
        for (k, v) in &headers {
            if k == "authorization" && !same_host {
                continue;
            }
            request = request.header(k.as_str(), v.as_str());
        }
        if let Some(b) = &body {
            request = request.body(b.clone());
        }
        let mut response = request.send().await.map_err(|e| if e.is_timeout() { "the request timed out".to_string() } else { format!("couldn't connect to {host}") })?;
        let status = response.status();
        if status.is_redirection() {
            let Some(next) = response.headers().get("location").and_then(|l| l.to_str().ok()).and_then(|l| parsed.join(l).ok()) else {
                return Err("the server sent a broken redirect".into());
            };
            url = next.to_string();
            if status.as_u16() == 303 || ((status.as_u16() == 301 || status.as_u16() == 302) && method == "POST") {
                method = "GET".into();
                body = None;
            }
            continue;
        }
        if method != "HEAD" && response.content_length().is_some_and(|l| l as usize > RESPONSE_LIMIT) {
            return Err("the response is larger than 5 MB".into());
        }
        let mut out_headers = Map::new();
        for (k, v) in response.headers() {
            if k.as_str() == "set-cookie" {
                continue;
            }
            if let Ok(v) = v.to_str() {
                out_headers.insert(k.as_str().to_owned(), Value::String(v.chars().take(2000).collect()));
            }
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "the download was interrupted".to_string())? {
            if bytes.len() + chunk.len() > RESPONSE_LIMIT {
                return Err("the response is larger than 5 MB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(json!({
            "status": status.as_u16(),
            "statusText": status.canonical_reason().unwrap_or_default(),
            "url": url,
            "headers": out_headers,
            "body": String::from_utf8_lossy(&bytes),
        })
        .to_string());
    }
    Err("too many redirects".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_addresses() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.1.1", "172.16.0.5", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:192.168.0.1"] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "142.250.72.14", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn refuses_hosts_outside_the_list() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let hosts = vec!["example.com".to_owned()];
        let err = rt.block_on(fetch("https://evil.test/x".into(), "GET".into(), Vec::new(), None, hosts.clone())).unwrap_err();
        assert!(err.contains("only connect"));
        let err = rt.block_on(fetch("http://example.com/x".into(), "GET".into(), Vec::new(), None, hosts)).unwrap_err();
        assert!(err.contains("only connect"));
    }
}
