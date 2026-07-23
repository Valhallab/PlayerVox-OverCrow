use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const MEDIA: u8 = 1 << 0;
const WORLDSTATE: u8 = 1 << 1;
const MARKET: u8 = 1 << 2;

#[derive(Clone, Default)]
pub struct ProviderReadiness {
    bits: Arc<AtomicU8>,
}

impl ProviderReadiness {
    pub fn mark_media(&self) {
        self.bits.fetch_or(MEDIA, Ordering::Release);
    }

    pub fn mark_worldstate(&self) {
        self.bits.fetch_or(WORLDSTATE, Ordering::Release);
    }

    pub fn mark_market(&self) {
        self.bits.fetch_or(MARKET, Ordering::Release);
    }

    pub fn take(&self) -> ReadyProviders {
        ReadyProviders(self.bits.swap(0, Ordering::AcqRel))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyProviders(u8);

impl ReadyProviders {
    pub fn media(self) -> bool {
        self.0 & MEDIA != 0
    }

    pub fn worldstate(self) -> bool {
        self.0 & WORLDSTATE != 0
    }

    pub fn market(self) -> bool {
        self.0 & MARKET != 0
    }

    #[cfg(test)]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}
