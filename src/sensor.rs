//! 行为感知：只读系统空闲时长与前台进程名，聚合成分钟桶。
//!
//! 隐私边界：
//! - 不读取窗口标题、不记录键击内容、不截屏、不联网；
//! - 应用名只存在于内存（用于“探索度”），不会写入磁盘；
//! - 落盘的只有 活跃秒数 / 切换次数 / 有输入的秒数。

use std::collections::HashMap;

use crate::signature::{self, Bucket, Signature};

/// 一次采样的原始结果。
#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub idle_ms: u64,
    pub app: Option<String>,
    /// 这一秒内是否有新的键鼠输入（距离上次输入 < 1.5 秒）。
    pub input: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// 一次真正的休息（离开 >= 3 分钟）。
    Break,
    /// 一个分钟桶结算完成。
    MinuteRolled,
}

/// 把逐秒采样累积成分钟桶，并维护连续活跃时长。
pub struct Tracker {
    pub buckets: Vec<Bucket>,
    pub cur_minute: u64,
    pub cur_active_sec: u32,
    pub cur_switches: u32,
    pub cur_input_sec: u32,
    last_app: Option<String>,
    apps: HashMap<String, u64>,
    pub streak_sec: u64,
    pub last_break_ms: u64,
    rest_recorded: bool,
    started: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            cur_minute: 0,
            cur_active_sec: 0,
            cur_switches: 0,
            cur_input_sec: 0,
            last_app: None,
            apps: HashMap::new(),
            streak_sec: 0,
            last_break_ms: 0,
            rest_recorded: true,
            started: false,
        }
    }
}

impl Tracker {
    /// 推进一秒。
    pub fn step(&mut self, now_ms: u64, s: &Sample) -> Vec<Event> {
        let mut events = Vec::new();
        let now_min = now_ms / 60_000;

        if !self.started {
            self.started = true;
            self.cur_minute = now_min;
            self.last_break_ms = now_ms;
        }

        // 分钟结算。睡眠/暂停造成的空档直接跳过，不补零桶。
        if now_min > self.cur_minute {
            self.buckets.push(Bucket {
                t: self.cur_minute,
                active: self.cur_active_sec.min(60),
                switches: self.cur_switches,
                input: self.cur_input_sec.min(60),
            });
            self.cur_minute = now_min;
            self.cur_active_sec = 0;
            self.cur_switches = 0;
            self.cur_input_sec = 0;
            events.push(Event::MinuteRolled);
        }

        // 前台应用切换：只留名字，不落盘。
        if let Some(app) = &s.app {
            let changed = self.last_app.as_deref() != Some(app.as_str());
            if changed {
                self.cur_switches += 1;
                self.last_app = Some(app.clone());
            }
            self.apps.insert(app.clone(), now_ms);
        }

        self.apps
            .retain(|_, seen| now_ms.saturating_sub(*seen) < 45 * 60_000);
        self.buckets
            .retain(|b| now_ms.saturating_sub(b.t * 60_000) < 3 * 3600 * 1000);

        let active = s.idle_ms < 60_000;
        if active {
            self.cur_active_sec += 1;
            self.streak_sec += 1;
            self.rest_recorded = false;
        } else if s.idle_ms >= 180_000 {
            if !self.rest_recorded {
                self.rest_recorded = true;
                self.last_break_ms = now_ms;
                self.streak_sec = 0;
                events.push(Event::Break);
            }
        }

        if s.input {
            self.cur_input_sec += 1;
        }

        events
    }

    /// 含当前未结算分钟的桶视图。
    pub fn view_buckets(&self) -> Vec<Bucket> {
        let mut v = self.buckets.clone();
        v.push(Bucket {
            t: self.cur_minute,
            active: self.cur_active_sec.min(60),
            switches: self.cur_switches,
            input: self.cur_input_sec.min(60),
        });
        v
    }

    /// 最近 n 分钟的活跃秒数，用于自检面板。
    pub fn recent_active(&self, n: usize, now_ms: u64) -> Vec<u8> {
        let now_min = now_ms / 60_000;
        let span = n as u64;
        let from = now_min.saturating_sub(span.saturating_sub(1));
        let mut map: HashMap<u64, u32> = self
            .buckets
            .iter()
            .filter(|b| b.t >= from && b.t <= now_min)
            .map(|b| (b.t, b.active))
            .collect();
        map.insert(self.cur_minute, self.cur_active_sec);
        (0..span)
            .map(|i| map.get(&(from + i)).copied().unwrap_or(0).min(60) as u8)
            .collect()
    }

    /// 睡眠/休眠唤醒后清掉连续状态，避免把中断算成连续工作。
    pub fn reset_after_gap(&mut self, now_ms: u64) {
        self.cur_minute = now_ms / 60_000;
        self.cur_active_sec = 0;
        self.cur_switches = 0;
        self.cur_input_sec = 0;
        self.streak_sec = 0;
        self.rest_recorded = true;
        self.last_break_ms = now_ms;
        self.last_app = None;
        self.started = true;
    }

    pub fn signature(&self, now_ms: u64) -> Signature {
        signature::compute(
            &self.view_buckets(),
            now_ms,
            self.streak_sec,
            self.last_break_ms,
            self.apps.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: u64 = 60_000;

    #[test]
    fn rolls_minutes_and_counts_switches() {
        let mut t = Tracker::default();
        let base = 1_700_000_000 * M;
        for i in 0..120u64 {
            let s = Sample {
                idle_ms: 0,
                app: Some(if i < 60 { "editor".into() } else { "browser".into() }),
                input: true,
            };
            t.step(base + i * 1000, &s);
        }
        assert_eq!(t.cur_minute, base / M + 1);
        assert_eq!(t.buckets.len(), 1);
        assert_eq!(t.buckets[0].active, 60);
        assert_eq!(t.buckets[0].input, 60);
        assert!(t.cur_switches >= 1);
    }

    #[test]
    fn detects_break_after_three_idle_minutes() {
        let mut t = Tracker::default();
        let base = 1_700_000_000 * M;
        let mut breaks = 0;
        for i in 0..600u64 {
            let idle = if i < 120 { 0 } else { 200_000 };
            let s = Sample {
                idle_ms: idle,
                app: Some("editor".into()),
                input: idle == 0,
            };
            for e in t.step(base + i * 1000, &s) {
                if e == Event::Break {
                    breaks += 1;
                }
            }
        }
        assert_eq!(breaks, 1);
        assert_eq!(t.streak_sec, 0);
    }

    #[test]
    fn recent_active_returns_window() {
        let mut t = Tracker::default();
        let base = 1_700_000_000 * M;
        for i in 0..90u64 {
            t.step(
                base + i * 1000,
                &Sample {
                    idle_ms: 0,
                    app: Some("editor".into()),
                    input: true,
                },
            );
        }
        let v = t.recent_active(60, base + 90_000);
        assert_eq!(v.len(), 60);
        assert_eq!(*v.last().unwrap(), 30);
    }

    #[test]
    fn reset_after_gap_clears_streak() {
        let mut t = Tracker::default();
        let base = 1_700_000_000 * M;
        for i in 0..300u64 {
            t.step(
                base + i * 1000,
                &Sample {
                    idle_ms: 0,
                    app: Some("editor".into()),
                    input: true,
                },
            );
        }
        assert!(t.streak_sec >= 300);
        let resume = base + 8 * 3600 * 1000;
        t.reset_after_gap(resume);
        assert_eq!(t.streak_sec, 0);
        assert_eq!(t.cur_active_sec, 0);
        assert_eq!(t.cur_minute, resume / M);
    }
}
