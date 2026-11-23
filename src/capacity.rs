use crate::{protocol::game::privateapi::GetGameServerCapacityResponse, rpc::Failure};
use std::collections::BTreeMap;
pub const BATTLES_PER_THREAD: u32 = 16;
const RESERVED_CPU_THREADS: usize = 4;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Scheduled,
    InProgress,
    Resolving,
    PreloadFailed,
    Completed,
}
impl Phase {
    fn counts(self) -> bool {
        matches!(self, Self::Scheduled | Self::InProgress | Self::Resolving)
    }
}
pub struct Registry {
    instance: String,
    threads: u32,
    ready: bool,
    draining: bool,
    battles: BTreeMap<u64, (usize, Phase)>,
}
impl Registry {
    pub fn new(instance: String, cpu_threads: usize) -> Result<Self, Failure> {
        if instance.is_empty() || cpu_threads == 0 {
            return Err(Failure::BAD_REQUEST);
        }
        // Dedicated thread count = max(1, CPU threads - 4).
        let threads = u32::try_from(cpu_threads.saturating_sub(RESERVED_CPU_THREADS).max(1))
            .map_err(|_| Failure::BAD_REQUEST)?;
        threads
            .checked_mul(BATTLES_PER_THREAD)
            .ok_or(Failure::BAD_REQUEST)?;
        Ok(Self {
            instance,
            threads,
            ready: false,
            draining: false,
            battles: BTreeMap::new(),
        })
    }
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
    }
    pub fn drain(&mut self) {
        self.draining = true;
    }
    pub fn worker(&self, id: u64) -> Option<usize> {
        self.battles.get(&id).map(|v| v.0)
    }
    pub fn phase(&self, id: u64) -> Option<Phase> {
        self.battles.get(&id).map(|v| v.1)
    }
    pub fn reserve(&mut self, id: u64) -> Result<usize, Failure> {
        if let Some((worker, _)) = self.battles.get(&id) {
            return Ok(*worker);
        }
        if id == 0 {
            return Err(Failure::BAD_REQUEST);
        }
        if !self.ready || self.draining {
            return Err(Failure::UNAVAILABLE);
        }
        let counts = self.counts();
        let (worker, count) = counts
            .iter()
            .enumerate()
            .min_by_key(|(worker, count)| (**count, *worker))
            .ok_or(Failure::UNAVAILABLE)?;
        if *count == BATTLES_PER_THREAD {
            return Err(Failure::UNAVAILABLE);
        }
        self.battles.insert(id, (worker, Phase::Scheduled));
        Ok(worker)
    }
    pub fn transition(&mut self, id: u64, next: Phase) -> Result<(), Failure> {
        let entry = self.battles.get_mut(&id).ok_or(Failure::NOT_FOUND)?;
        if entry.1 == next {
            return Ok(());
        }
        if !matches!(
            (entry.1, next),
            (Phase::Scheduled, Phase::InProgress | Phase::PreloadFailed)
                | (Phase::InProgress, Phase::Resolving)
                | (Phase::Resolving, Phase::Completed)
        ) {
            return Err(Failure::CONFLICT);
        }
        entry.1 = next;
        Ok(())
    }
    pub fn discard(&mut self, id: u64) {
        self.battles.remove(&id);
    }
    fn counts(&self) -> Vec<u32> {
        let mut counts = vec![0; self.threads as usize];
        for (worker, phase) in self.battles.values() {
            if phase.counts() {
                counts[*worker] += 1;
            }
        }
        counts
    }
    pub fn capacity(&self) -> GetGameServerCapacityResponse {
        let counts = self.counts();
        let owned: u32 = counts.iter().sum();
        let accepts = self.ready && !self.draining;
        // Available battles = total threads * 16 - owned(scheduled + in_progress + resolving).
        let available = if accepts {
            self.threads * BATTLES_PER_THREAD - owned
        } else {
            0
        };
        GetGameServerCapacityResponse {
            game_server_instance_id: self.instance.clone(),
            accept_new_guild_battle: accepts,
            owned_guild_battle_count: owned,
            available_guild_battle_count: available,
            available_guild_battle_thread_count: counts.iter().filter(|count| **count == 0).count()
                as u32,
            total_guild_battle_thread_count: self.threads,
        }
    }
}
