pub mod daemon;
pub mod heal;
pub mod lifecycle;
pub mod orchestrator;

pub const EXO_DEFAULT_PORT: u16 = 52415;
/// Fixed libp2p port so another Mac can dial us when mDNS is blocked.
pub const EXO_LIBP2P_PORT: u16 = 52416;
pub const EXO_LOCAL_ORIGIN: &str = "http://127.0.0.1:52415";
