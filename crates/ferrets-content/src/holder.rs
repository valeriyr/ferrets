//! The holder vocabulary: what an entity holds others as.

use crate::{gateway::GatewayDef, transport::TransporterDef};

/// What instances of a type hold others as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// A transporter: passengers ride aboard it.
    Transporter(TransporterDef),
    /// A gateway: bodies enter it into what it joins, or cross through it.
    Gateway(GatewayDef),
}

impl Holder {
    /// The transporter it holds as, or `None` for a gateway.
    pub fn transporter(&self) -> Option<&TransporterDef> {
        match self {
            Holder::Transporter(transporter) => Some(transporter),
            Holder::Gateway(_) => None,
        }
    }

    /// The gateway it holds as, or `None` for a transporter.
    pub fn gateway(&self) -> Option<&GatewayDef> {
        match self {
            Holder::Gateway(gateway) => Some(gateway),
            Holder::Transporter(_) => None,
        }
    }
}
