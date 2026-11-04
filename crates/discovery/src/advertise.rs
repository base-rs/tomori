// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::net::IpAddr;
use std::time::Duration;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use tomori_protocol::PROTOCOL_VERSION;

use crate::record::{INSTANCE_LIMIT, properties, truncate};
use crate::{Counts, Error, SERVICE_TYPE};

/// How long dropping an [`Advertisement`] waits for its goodbye packets, so that browsers drop the server at once rather than when its records expire.
const GOODBYE_TIMEOUT: Duration = Duration::from_secs(1);

/// A running advertisement; dropping it withdraws the service.
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
    service: Service,
}

/// What a service is built from, so that it can be registered again with other counts.
struct Service {
    name: String,
    instance: String,
    host: String,
    port: u16,
    address: IpAddr,
    fingerprint: Option<String>,
}

impl Service {
    fn info(&self, counts: Counts) -> Result<ServiceInfo, mdns_sd::Error> {
        let properties = properties(
            &self.name,
            PROTOCOL_VERSION,
            counts,
            self.fingerprint.as_deref(),
        );
        if self.address.is_unspecified() {
            let mut service = ServiceInfo::new(
                SERVICE_TYPE,
                &self.instance,
                &self.host,
                (),
                self.port,
                properties.as_slice(),
            )?
            .enable_addr_auto();
            if self.address.is_ipv4() {
                service.set_interfaces(vec![IfKind::IPv4]);
            }
            Ok(service)
        } else {
            let mut service = ServiceInfo::new(
                SERVICE_TYPE,
                &self.instance,
                &self.host,
                self.address,
                self.port,
                properties.as_slice(),
            )?;
            service.set_interfaces(vec![IfKind::Addr(self.address)]);
            Ok(service)
        }
    }
}

impl std::fmt::Debug for Advertisement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Advertisement")
            .field("fullname", &self.fullname)
            .finish_non_exhaustive()
    }
}

impl Advertisement {
    /// The requested instance name. If another service on the network already uses it, the responder renames this one (`Name (2)`); the TXT `name` keeps the display name.
    pub fn instance(&self) -> &str {
        &self.service.instance
    }

    /// Advertise `counts` in place of the previous ones, without blocking. The responder announces the changed TXT record, and browsers report the server as [`crate::Event::Found`] again with the new counts.
    ///
    /// `mdns_sd` probes a changed record before announcing it, which takes about a second, and a browser flushes a cached record only once it has held it for a second (RFC 6762, section 10.2), so a count that changes back within a second can leave browsers showing the one in between. Update at most every couple of seconds, coalescing the changes in between.
    pub fn update(&self, counts: Counts) -> Result<(), Error> {
        // Registering the same service again re-announces it, under the instance and host names the responder settled on.
        self.daemon.register(self.service.info(counts)?)?;
        Ok(())
    }
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        if let Ok(status) = self.daemon.unregister(&self.fullname) {
            let _ = status.recv_timeout(GOODBYE_TIMEOUT);
        }
        if let Ok(status) = self.daemon.shutdown() {
            let _ = status.recv_timeout(GOODBYE_TIMEOUT);
        }
    }
}

/// Advertise a Tomori server called `name`, listening on `port` at `address`, with its current `counts` and, if it serves TLS, its certificate's `fingerprint`, until the returned [`Advertisement`] is dropped.
///
/// `address` is the address the server is bound to. If it is unspecified (`0.0.0.0` or `::`), the server is advertised on every interface with that address family's addresses (`::` includes IPv4, as dual-stack sockets accept both), and the addresses follow interface changes. Otherwise only that address is advertised, on its interface.
///
/// The responder runs on its own thread; this does not block beyond starting it.
pub fn advertise(
    name: &str,
    port: u16,
    address: IpAddr,
    counts: Counts,
    fingerprint: Option<String>,
) -> Result<Advertisement, Error> {
    let name = match name.trim() {
        "" => "Tomori",
        name => name,
    };
    let instance = truncate(name, INSTANCE_LIMIT).to_owned();
    let service = Service {
        name: name.to_owned(),
        host: host_name(&instance),
        instance,
        port,
        address,
        fingerprint,
    };
    let daemon = ServiceDaemon::new()?;
    if address.is_unspecified() {
        // Loopback interfaces only reach this computer, and may carry many alias addresses.
        daemon.disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;
    }
    let info = service.info(counts)?;
    let fullname = info.get_fullname().to_owned();
    if let Err(error) = daemon.register(info) {
        let _ = daemon.shutdown();
        return Err(error.into());
    }
    Ok(Advertisement {
        daemon,
        fullname,
        service,
    })
}

/// A multicast DNS host name for the service's address records, derived from the instance name. It is distinct from the computer's own `.local` name, which the system responder owns.
fn host_name(instance: &str) -> String {
    let mut label = String::from("tomori");
    let mut dash = true;
    for character in instance.chars() {
        if character.is_ascii_alphanumeric() {
            if dash {
                label.push('-');
            }
            label.push(character.to_ascii_lowercase());
            dash = false;
        } else {
            dash = true;
        }
    }
    if label.starts_with("tomori-tomori") {
        label.replace_range(.."tomori-".len(), "");
    }
    label.truncate(INSTANCE_LIMIT);
    let label = label.trim_end_matches('-');
    format!("{label}.local.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_a_host_name_label() {
        assert_eq!(
            host_name("Samuel's MacBook Pro"),
            "tomori-samuel-s-macbook-pro.local."
        );
        assert_eq!(host_name("スタジオ"), "tomori.local.");
        assert_eq!(host_name("Tomori Test"), "tomori-test.local.");
        let long = host_name(&"a".repeat(100));
        assert_eq!(long.len(), INSTANCE_LIMIT + ".local.".len());
    }
}
