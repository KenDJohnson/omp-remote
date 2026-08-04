use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_identifier {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(IdentifierError($label));
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

string_identifier!(AgentId, "agent ID");
string_identifier!(RunId, "run ID");
string_identifier!(LeaseHolderId, "lease holder ID");
string_identifier!(ServerId, "server ID");
string_identifier!(ConnectionId, "connection ID");
string_identifier!(DeviceId, "device ID");
string_identifier!(PairingId, "pairing ID");
string_identifier!(RequestId, "request ID");
string_identifier!(OperationId, "operation ID");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentifierError(&'static str);

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} cannot be empty", self.0)
    }
}

impl std::error::Error for IdentifierError {}
