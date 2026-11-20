//! IP bans at the door: an address or CIDR block ([`Network`]) and the check the accept
//! loop asks of the server before a telnet connection gets the login screen ([`IpFilter`]).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// Asked once per accepted telnet connection, before anything is sent: `true` closes it
/// at once. The rlogin listener never asks (its callers all come from the BBS).
pub trait IpFilter: Send + Sync + fmt::Debug {
    /// Whether connections from `ip` are refused.
    fn refuses(&self, ip: IpAddr) -> bool;
}

/// An address or a CIDR block, IPv4 or IPv6. An IPv4-mapped IPv6 address
/// (`::ffff:203.0.113.7`) is taken as the IPv4 address it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Network {
    /// The first address of the block (host bits cleared).
    addr: IpAddr,
    /// The prefix length: 0..=32 for IPv4, 0..=128 for IPv6.
    prefix: u8,
}

/// Why a [`Network`] did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not an IP address or CIDR block (203.0.113.7, 203.0.113.0/24, 2001:db8::/32)")]
pub struct BadNetwork(pub String);

/// The address with an IPv4-mapped IPv6 address taken as IPv4.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

fn mask_v4(a: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let bits = u32::from(a);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Ipv4Addr::from(bits & mask)
}

fn mask_v6(a: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let bits = u128::from(a);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    Ipv6Addr::from(bits & mask)
}

impl Network {
    /// The block of `prefix` bits around `addr`; `None` when the prefix is too long.
    pub fn new(addr: IpAddr, prefix: u8) -> Option<Self> {
        Some(match canonical(addr) {
            IpAddr::V4(a) if prefix <= 32 => Self {
                addr: IpAddr::V4(mask_v4(a, prefix)),
                prefix,
            },
            IpAddr::V6(a) if prefix <= 128 => Self {
                addr: IpAddr::V6(mask_v6(a, prefix)),
                prefix,
            },
            _ => return None,
        })
    }

    /// One address.
    pub fn host(addr: IpAddr) -> Self {
        let addr = canonical(addr);
        let prefix = if addr.is_ipv4() { 32 } else { 128 };
        Self { addr, prefix }
    }

    /// Whether `ip` is in the block.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, canonical(ip)) {
            (IpAddr::V4(net), IpAddr::V4(a)) => mask_v4(a, self.prefix) == net,
            (IpAddr::V6(net), IpAddr::V6(a)) => mask_v6(a, self.prefix) == net,
            _ => false,
        }
    }
}

impl FromStr for Network {
    type Err = BadNetwork;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadNetwork(s.to_string());
        let s = s.trim();
        match s.split_once('/') {
            None => s.parse::<IpAddr>().map(Self::host).map_err(|_| bad()),
            Some((addr, prefix)) => {
                let addr = addr.parse::<IpAddr>().map_err(|_| bad())?;
                // a mapped address's prefix counts the IPv6 bits
                let mut prefix: u8 = prefix.parse().map_err(|_| bad())?;
                if let IpAddr::V6(v6) = addr
                    && v6.to_ipv4_mapped().is_some()
                {
                    prefix = prefix.checked_sub(96).ok_or_else(bad)?;
                }
                Self::new(addr, prefix).ok_or_else(bad)
            }
        }
    }
}

/// The canonical form: `203.0.113.7` for one address, `203.0.113.0/24` for a block.
impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let full = if self.addr.is_ipv4() { 32 } else { 128 };
        if self.prefix == full {
            write!(f, "{}", self.addr)
        } else {
            write!(f, "{}/{}", self.addr, self.prefix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn parses_hosts_and_blocks_to_their_canonical_form() {
        let canon = |s: &str| s.parse::<Network>().unwrap().to_string();
        assert_eq!(canon("203.0.113.7"), "203.0.113.7");
        assert_eq!(canon("203.0.113.7/32"), "203.0.113.7");
        assert_eq!(canon("203.0.113.77/24"), "203.0.113.0/24");
        assert_eq!(canon(" 10.1.2.3/8 "), "10.0.0.0/8");
        assert_eq!(canon("0.0.0.0/0"), "0.0.0.0/0");
        assert_eq!(canon("2001:db8::1"), "2001:db8::1");
        assert_eq!(canon("2001:db8:1:2::5/32"), "2001:db8::/32");
        assert_eq!(canon("::ffff:203.0.113.7"), "203.0.113.7");
        assert_eq!(canon("::ffff:203.0.113.7/120"), "203.0.113.0/24");
        for bad in [
            "",
            "nope",
            "203.0.113.7/33",
            "2001:db8::/129",
            "1.2.3.4/x",
            "::ffff:1.2.3.4/90",
        ] {
            assert!(bad.parse::<Network>().is_err(), "{bad}");
        }
    }

    #[test]
    fn contains_checks_the_prefix_bits() {
        let net: Network = "203.0.113.0/24".parse().unwrap();
        assert!(net.contains(ip("203.0.113.0")));
        assert!(net.contains(ip("203.0.113.255")));
        assert!(net.contains(ip("::ffff:203.0.113.9")));
        assert!(!net.contains(ip("203.0.114.1")));
        assert!(!net.contains(ip("2001:db8::1")));
        let host: Network = "198.51.100.4".parse().unwrap();
        assert!(host.contains(ip("198.51.100.4")) && !host.contains(ip("198.51.100.5")));
        let v6: Network = "2001:db8::/32".parse().unwrap();
        assert!(v6.contains(ip("2001:db8:ffff::1")) && !v6.contains(ip("2001:db9::1")));
        let all: Network = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains(ip("8.8.8.8")) && !all.contains(ip("::1")));
    }
}
