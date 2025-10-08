use ipnetwork::IpNetwork;
use log::{info, warn};
use std::net::IpAddr;
use std::str::FromStr;

/// Official Telegram webhook IP ranges
/// Source: https://core.telegram.org/resources/cidr.txt
const TELEGRAM_IP_RANGES: &[&str] = &[
    // IPv4 ranges
    "91.108.56.0/22",
    "91.108.4.0/22",
    "91.108.8.0/22",
    "91.108.16.0/22",
    "91.108.12.0/22",
    "149.154.160.0/20",
    "91.105.192.0/23",
    "91.108.20.0/22",
    "185.76.151.0/24",
    // IPv6 ranges
    "2001:b28:f23d::/48",
    "2001:b28:f23f::/48",
    "2001:67c:4e8::/48",
    "2001:b28:f23c::/48",
    "2a0a:f280::/32",
];

/// Validates if an IP address is from a Telegram server
pub fn is_telegram_ip(ip_str: &str) -> bool {
    let ip = match IpAddr::from_str(ip_str) {
        Ok(addr) => addr,
        Err(e) => {
            warn!("Failed to parse IP address: {}", e);
            return false;
        }
    };

    for range_str in TELEGRAM_IP_RANGES {
        let network = match IpNetwork::from_str(range_str) {
            Ok(net) => net,
            Err(e) => {
                warn!("Failed to parse IP range '{}': {}", range_str, e);
                continue;
            }
        };

        if network.contains(ip) {
            info!("IP matched Telegram range");
            return true;
        }
    }

    warn!("IP is NOT from Telegram servers");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_telegram_ipv4() {
        assert!(is_telegram_ip("91.108.56.1"));
        assert!(is_telegram_ip("149.154.167.197"));
    }

    #[test]
    fn test_invalid_telegram_ip() {
        assert!(!is_telegram_ip("1.2.3.4"));
        assert!(!is_telegram_ip("192.168.1.1"));
    }

    #[test]
    fn test_valid_telegram_ipv6() {
        assert!(is_telegram_ip("2001:b28:f23d::1"));
    }
}
