pub mod conclave {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/conclave.v1.rs"));
    }
}

pub use conclave::v1::*;
