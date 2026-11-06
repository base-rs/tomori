// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Starting the multicast DNS responder and querier, whose failures come from the system, such as running out of descriptors, and cannot be made to happen in tests.
use mdns_sd::{IfKind, Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::{Error, SERVICE_TYPE};

/// A responder announcing `info`, on every interface but loopback if `everywhere`.
pub fn responder(info: ServiceInfo, everywhere: bool) -> Result<ServiceDaemon, Error> {
    let daemon = ServiceDaemon::new()?;
    if everywhere {
        // Loopback interfaces only reach this computer, and may carry many alias addresses.
        daemon.disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;
    }
    if let Err(error) = daemon.register(info) {
        let _ = daemon.shutdown();
        return Err(error.into());
    }
    Ok(daemon)
}

/// A querier browsing for Tomori servers, with its events.
pub fn querier() -> Result<(ServiceDaemon, Receiver<ServiceEvent>), Error> {
    let daemon = ServiceDaemon::new()?;
    match daemon.browse(SERVICE_TYPE) {
        Ok(events) => Ok((daemon, events)),
        Err(error) => {
            let _ = daemon.shutdown();
            Err(error.into())
        }
    }
}
