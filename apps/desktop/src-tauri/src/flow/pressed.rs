pub const MODIFIER_IDS: [u32; 8] = [0x2A, 0x36, 0x1D, 0x11D, 0x38, 0x138, 0x15B, 0x15C];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Local,
    Forward,
    Drop,
}

pub trait Slots: Copy + Default {
    const REPEATS: bool;
    fn has(&self, id: u32) -> bool;
    fn put(&mut self, id: u32) -> bool;
    fn take(&mut self, id: u32) -> bool;
    fn any(&self) -> bool;
    fn union(&mut self, other: &Self);
    fn remove(&mut self, other: &Self);
}

const KEY_WORDS: usize = 16;
const UNMAPPED_BASE: u32 = 0x1000;
const MAPPED_SLOTS: u32 = 0x200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeySet([u64; KEY_WORDS]);
impl KeySet {
    fn slot(id: u32) -> Option<(usize, u64)> {
        let index = match id {
            0..MAPPED_SLOTS => id,
            UNMAPPED_BASE..0x1200 => id - UNMAPPED_BASE + MAPPED_SLOTS,
            _ => return None,
        } as usize;
        Some((index / 64, 1u64 << (index % 64)))
    }
    fn id(index: usize) -> u32 {
        let index = index as u32;
        if index < MAPPED_SLOTS {
            index
        } else {
            index - MAPPED_SLOTS + UNMAPPED_BASE
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(word, bits)| {
            (0..64)
                .filter(move |bit| bits & (1u64 << bit) != 0)
                .map(move |bit| Self::id(word * 64 + bit))
        })
    }
}
impl Slots for KeySet {
    const REPEATS: bool = true;
    fn has(&self, id: u32) -> bool {
        Self::slot(id).is_some_and(|(word, bit)| self.0[word] & bit != 0)
    }
    fn put(&mut self, id: u32) -> bool {
        let Some((word, bit)) = Self::slot(id) else {
            return false;
        };
        let fresh = self.0[word] & bit == 0;
        self.0[word] |= bit;
        fresh
    }
    fn take(&mut self, id: u32) -> bool {
        let Some((word, bit)) = Self::slot(id) else {
            return false;
        };
        let held = self.0[word] & bit != 0;
        self.0[word] &= !bit;
        held
    }
    fn any(&self) -> bool {
        self.0.iter().any(|w| *w != 0)
    }
    fn union(&mut self, other: &Self) {
        for (word, extra) in self.0.iter_mut().zip(other.0) {
            *word |= extra;
        }
    }
    fn remove(&mut self, other: &Self) {
        for (word, gone) in self.0.iter_mut().zip(other.0) {
            *word &= !gone;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonSet(u8);
impl ButtonSet {
    fn bit(id: u32) -> u8 {
        1u8 << id.min(7)
    }
    pub fn iter(&self) -> impl Iterator<Item = u8> + '_ {
        (0..8u8).filter(move |i| self.0 & (1 << i) != 0)
    }
}
impl Slots for ButtonSet {
    const REPEATS: bool = false;
    fn has(&self, id: u32) -> bool {
        self.0 & Self::bit(id) != 0
    }
    fn put(&mut self, id: u32) -> bool {
        let fresh = !self.has(id);
        self.0 |= Self::bit(id);
        fresh
    }
    fn take(&mut self, id: u32) -> bool {
        let held = self.has(id);
        self.0 &= !Self::bit(id);
        held
    }
    fn any(&self) -> bool {
        self.0 != 0
    }
    fn union(&mut self, other: &Self) {
        self.0 |= other.0;
    }
    fn remove(&mut self, other: &Self) {
        self.0 &= !other.0;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Tracker<S: Slots> {
    held: S,
    local: S,
    remote: S,
    released: S,
}
impl<S: Slots> Tracker<S> {
    pub fn held(&self, id: u32) -> bool {
        self.held.has(id)
    }
    pub fn any_held(&self) -> bool {
        self.held.any()
    }
    #[cfg(test)]
    pub fn local_down(&self, id: u32) -> bool {
        self.local.has(id)
    }
    #[cfg(test)]
    pub fn forwarded_down(&self, id: u32) -> bool {
        self.remote.has(id)
    }
    #[cfg(test)]
    pub fn awaiting_release(&self, id: u32) -> bool {
        self.released.has(id)
    }
    pub fn route(&mut self, id: u32, down: bool, forwarding: bool) -> Route {
        if down {
            self.held.put(id);
        } else {
            self.held.take(id);
        }
        match (forwarding, down) {
            (true, true) => {
                if self.remote.has(id) {
                    return Route::Drop;
                }
                if self.local.has(id) {
                    if S::REPEATS {
                        return Route::Drop;
                    }
                    self.local.take(id);
                }
                self.released.take(id);
                self.remote.put(id);
                Route::Forward
            }
            (true, false) => {
                if self.local.take(id) {
                    Route::Local
                } else if self.remote.take(id) {
                    Route::Forward
                } else {
                    self.released.take(id);
                    Route::Drop
                }
            }
            (false, true) => {
                self.remote.take(id);
                self.released.take(id);
                self.local.put(id);
                Route::Local
            }
            (false, false) => {
                let remote = self.remote.take(id);
                let released = self.released.take(id);
                self.local.take(id);
                if remote || released {
                    Route::Drop
                } else {
                    Route::Local
                }
            }
        }
    }
    pub fn adopt_local(&mut self, id: u32) {
        if !self.remote.has(id) && !self.released.has(id) {
            self.local.put(id);
        }
    }
    /// Forgets a local press whose release the hook never saw, for example
    /// one released on the secure desktop. Presses owned by the remote are
    /// kept so their release still stays away from the local system.
    pub fn forget_local(&mut self, id: u32) {
        if !self.remote.has(id) && !self.released.has(id) {
            self.held.take(id);
            self.local.take(id);
        }
    }
    /// Forgets every local press, as `forget_local` does for one id.
    pub fn forget_all_local(&mut self) {
        let mut stale = self.local;
        stale.remove(&self.remote);
        stale.remove(&self.released);
        self.held.remove(&stale);
        self.local.remove(&stale);
    }
    pub fn begin(&mut self) {
        self.finish();
    }
    pub fn finish(&mut self) -> S {
        let remote = std::mem::take(&mut self.remote);
        self.released.union(&remote);
        remote
    }
}

pub type KeyTracker = Tracker<KeySet>;
pub type ButtonTracker = Tracker<ButtonSet>;

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: u32 = 0x38;
    const A: u32 = 0x1E;

    #[test]
    fn key_set_covers_mapped_and_unmapped_ids() {
        let mut set = KeySet::default();
        for id in [0x00, 0x1D, 0x11D, 0x1FF, 0x1000, 0x1033, 0x11FF] {
            assert!(set.put(id), "{id:#x}");
            assert!(set.has(id));
        }
        assert!(!set.put(0x1D));
        assert!(!set.put(0x200));
        assert!(!set.has(0x200));
        assert!(!set.put(0x5000));
        let ids: Vec<u32> = set.iter().collect();
        assert_eq!(ids, vec![0x00, 0x1D, 0x11D, 0x1FF, 0x1000, 0x1033, 0x11FF]);
        assert!(set.take(0x11D) && !set.take(0x11D));
        assert!(set.any());
    }

    #[test]
    fn key_pressed_locally_is_released_locally_while_forwarding() {
        let mut keys = KeyTracker::default();
        assert_eq!(keys.route(ALT, true, false), Route::Local);
        keys.begin();
        assert_eq!(keys.route(ALT, true, true), Route::Drop, "auto-repeat stays home");
        assert_eq!(keys.route(ALT, true, true), Route::Drop);
        assert_eq!(keys.route(ALT, false, true), Route::Local);
        assert!(!keys.local_down(ALT) && !keys.forwarded_down(ALT));
        assert_eq!(keys.route(ALT, true, true), Route::Forward);
        assert_eq!(keys.route(ALT, false, true), Route::Forward);
        assert!(keys.finish().iter().next().is_none());
    }

    #[test]
    fn key_up_without_forwarded_down_is_not_forwarded() {
        let mut keys = KeyTracker::default();
        keys.begin();
        assert_eq!(keys.route(A, false, true), Route::Drop);
        assert_eq!(keys.route(A, true, true), Route::Forward);
        assert_eq!(keys.route(A, true, true), Route::Drop, "repeat");
        assert_eq!(keys.route(A, false, true), Route::Forward);
        assert_eq!(keys.route(A, false, true), Route::Drop);
    }

    #[test]
    fn keys_held_on_the_remote_are_released_and_their_up_stays_away() {
        let mut keys = KeyTracker::default();
        keys.begin();
        assert_eq!(keys.route(ALT, true, true), Route::Forward);
        assert_eq!(keys.route(A, true, true), Route::Forward);
        let released: Vec<u32> = keys.finish().iter().collect();
        assert_eq!(released, vec![A, ALT]);
        assert!(keys.awaiting_release(ALT) && !keys.forwarded_down(ALT));
        assert!(keys.finish().iter().next().is_none(), "released only once");
        assert_eq!(keys.route(ALT, false, false), Route::Drop);
        assert!(!keys.awaiting_release(ALT));
        assert_eq!(keys.route(ALT, true, false), Route::Local);
        assert_eq!(keys.route(ALT, false, false), Route::Local);
        assert_eq!(keys.route(A, true, false), Route::Local, "repeat moves home");
        assert!(keys.local_down(A) && !keys.awaiting_release(A));
        assert_eq!(keys.route(A, false, false), Route::Local);
    }

    #[test]
    fn a_key_still_held_when_forwarding_resumes_is_forwarded_again() {
        let mut keys = KeyTracker::default();
        keys.begin();
        keys.route(ALT, true, true);
        keys.finish();
        keys.begin();
        assert_eq!(keys.route(ALT, true, true), Route::Forward);
        assert_eq!(keys.route(ALT, false, true), Route::Forward);
        assert!(!keys.awaiting_release(ALT) && !keys.forwarded_down(ALT));
    }

    #[test]
    fn forwarding_lost_without_finish_keeps_remote_keys_away_from_home() {
        let mut keys = KeyTracker::default();
        keys.begin();
        keys.route(ALT, true, true);
        assert_eq!(keys.route(ALT, false, false), Route::Drop);
        keys.begin();
        keys.route(A, true, true);
        keys.begin();
        assert!(keys.awaiting_release(A));
        assert_eq!(keys.route(A, false, true), Route::Drop);
    }

    #[test]
    fn adopted_modifiers_release_locally() {
        let mut keys = KeyTracker::default();
        keys.begin();
        keys.adopt_local(ALT);
        assert_eq!(keys.route(ALT, false, true), Route::Local);
        assert!(!keys.held(ALT));
        keys.route(A, true, true);
        keys.finish();
        keys.adopt_local(A);
        assert!(!keys.local_down(A), "never adopt a key the remote owns");
    }

    #[test]
    fn held_tracks_physical_state_in_every_mode() {
        let mut keys = KeyTracker::default();
        keys.route(ALT, true, false);
        assert!(keys.held(ALT) && keys.any_held());
        keys.route(ALT, false, true);
        assert!(!keys.held(ALT) && !keys.any_held());
        keys.route(A, true, true);
        assert!(keys.held(A));
        keys.finish();
        keys.route(A, false, false);
        assert!(!keys.held(A));
    }

    #[test]
    fn buttons_never_stick_on_either_side() {
        let mut buttons = ButtonTracker::default();
        assert_eq!(buttons.route(0, true, false), Route::Local);
        buttons.begin();
        assert_eq!(buttons.route(0, false, true), Route::Local, "drag across the edge");
        assert_eq!(buttons.route(2, true, true), Route::Forward);
        let released: Vec<u8> = buttons.finish().iter().collect();
        assert_eq!(released, vec![2]);
        assert_eq!(buttons.route(2, false, false), Route::Drop);
        assert_eq!(buttons.route(1, false, true), Route::Drop);
    }

    #[test]
    fn forgetting_local_presses_keeps_the_remote_ones() {
        let mut keys = KeyTracker::default();
        keys.route(ALT, true, false);
        keys.begin();
        keys.route(A, true, true);
        keys.forget_local(ALT);
        keys.forget_local(A);
        assert!(!keys.held(ALT) && !keys.local_down(ALT));
        assert!(keys.held(A) && keys.forwarded_down(A), "the remote owns A");
        assert_eq!(keys.route(ALT, true, true), Route::Forward, "not a repeat");
        keys.finish();
        keys.route(0x11D, true, false);
        keys.forget_all_local();
        assert!(!keys.held(0x11D) && !keys.local_down(0x11D));
        assert!(keys.held(A) && keys.awaiting_release(A));
        assert_eq!(keys.route(A, false, false), Route::Drop);
    }

    #[test]
    fn separate_side_buttons_never_take_each_others_release() {
        let mut buttons = ButtonTracker::default();
        assert_eq!(buttons.route(3, true, false), Route::Local);
        buttons.begin();
        assert_eq!(buttons.route(4, true, true), Route::Forward);
        assert_eq!(buttons.route(4, false, true), Route::Forward);
        assert_eq!(buttons.route(3, false, true), Route::Local);
    }

    #[test]
    fn a_stale_local_button_does_not_trap_the_remote() {
        let mut buttons = ButtonTracker::default();
        buttons.route(0, true, false);
        buttons.begin();
        assert_eq!(buttons.route(0, true, true), Route::Forward);
        assert_eq!(buttons.route(0, false, true), Route::Forward);
        assert!(!buttons.local_down(0));
    }

    #[test]
    fn random_sequences_never_leave_anything_down() {
        let ids = [ALT, A, 0x11D, 0x1033];
        let mut seed: u64 = 0xF1_0E_5EED;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for _ in 0..300 {
            let mut keys = KeyTracker::default();
            let mut forwarding = false;
            let mut physical = [false; 4];
            let mut home = [false; 4];
            let mut away = [false; 4];
            for _ in 0..60 {
                match next(6) {
                    0 => {
                        if forwarding {
                            for id in keys.finish().iter() {
                                let i = ids.iter().position(|k| *k == id).unwrap();
                                away[i] = false;
                            }
                        } else {
                            keys.begin();
                        }
                        forwarding = !forwarding;
                    }
                    _ => {
                        let i = next(4) as usize;
                        let down = if physical[i] { next(3) == 0 } else { true };
                        physical[i] = down;
                        match keys.route(ids[i], down, forwarding) {
                            Route::Local => home[i] = down,
                            Route::Forward => away[i] = down,
                            Route::Drop => {}
                        }
                    }
                }
                for i in 0..4 {
                    assert!(!(home[i] && away[i]), "a key may be down on one side only");
                    if home[i] {
                        assert!(physical[i]);
                    }
                    if away[i] {
                        assert!(physical[i]);
                    }
                }
            }
            if forwarding {
                for id in keys.finish().iter() {
                    let i = ids.iter().position(|k| *k == id).unwrap();
                    away[i] = false;
                }
            }
            for i in 0..4 {
                if physical[i] {
                    let route = keys.route(ids[i], false, false);
                    match route {
                        Route::Local => home[i] = false,
                        Route::Forward => away[i] = false,
                        Route::Drop => {}
                    }
                }
            }
            assert_eq!(home, [false; 4]);
            assert_eq!(away, [false; 4]);
        }
    }
}
