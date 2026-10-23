//! 生态模型：把行为签名缓慢地翻译成生物的生理与环境参数。
//!
//! 所有变化都带惯性（指数缓动），所以不会因为一分钟的波动就突变；
//! 关闭程序期间按小时衰减，但累计的工作时长与阶段不会倒退。

use serde::{Deserialize, Serialize};

use crate::signature::{clamp01, Signature};

pub const STAGES: [(f64, &str); 4] = [
    (0.0, "孢子"),
    (45.0, "幼体"),
    (180.0, "成体"),
    (480.0, "繁盛"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Eco {
    pub vitality: f32,
    pub glow: f32,
    pub hue: f32,
    pub turbidity: f32,
    pub motility: f32,
    pub rhythm_t: f32,
    pub spots: f32,
    pub active_minutes: f64,
    pub focus_minutes: f64,
    pub last_tick_ms: u64,
    pub last_break_ms: u64,
    pub stage: u8,
}

impl Default for Eco {
    fn default() -> Self {
        Self {
            vitality: 0.55,
            glow: 0.12,
            hue: 0.15,
            turbidity: 0.35,
            motility: 0.3,
            rhythm_t: 0.3,
            spots: 0.0,
            active_minutes: 0.0,
            focus_minutes: 0.0,
            last_tick_ms: 0,
            last_break_ms: 0,
            stage: 0,
        }
    }
}

fn ease(cur: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if tau <= 0.0 {
        return target;
    }
    let k = 1.0 - (-dt / tau).exp();
    cur + (target - cur) * k
}

impl Eco {
    /// 进入程序时先处理离线时间。
    pub fn apply_offline(&mut self, now_ms: u64) {
        if self.last_tick_ms == 0 {
            self.last_tick_ms = now_ms;
            self.last_break_ms = now_ms;
            return;
        }
        let elapsed = now_ms.saturating_sub(self.last_tick_ms) as f32 / 1000.0;
        if elapsed < 120.0 {
            return;
        }
        self.vitality = ease(self.vitality, 0.30, elapsed, 6.0 * 3600.0);
        self.glow = ease(self.glow, 0.10, elapsed, 3.0 * 3600.0);
        self.turbidity = ease(self.turbidity, 0.28, elapsed, 5.0 * 3600.0);
        self.motility = ease(self.motility, 0.25, elapsed, 2.0 * 3600.0);
        self.spots *= 0.5;
        self.last_break_ms = now_ms;
        self.last_tick_ms = now_ms;
    }

    /// 每秒推进一次。返回阶段是否发生变化。
    pub fn tick(&mut self, sig: &Signature, dt_sec: f32, active_sec: f32, now_ms: u64) -> bool {
        self.last_tick_ms = now_ms;

        let glow_t = 0.12 + 0.88 * sig.focus;
        let vitality_t = clamp01(
            0.50 + 0.50 * sig.calm + 0.35 * sig.rhythm
                - 0.75 * sig.load
                - 0.25 * (1.0 - sig.active_ratio),
        );
        let turbidity_t = clamp01(0.12 + 0.55 * (1.0 - sig.calm) + 0.35 * sig.load);
        let motility_t = 0.20 + 0.65 * sig.active_ratio;
        let spots_t = clamp01(sig.load * 1.10 - 0.15);
        let hue_t = clamp01(sig.variety);

        self.glow = ease(self.glow, glow_t, dt_sec, 180.0);
        self.vitality = ease(self.vitality, vitality_t, dt_sec, 300.0);
        self.turbidity = ease(self.turbidity, turbidity_t, dt_sec, 300.0);
        self.motility = ease(self.motility, motility_t, dt_sec, 150.0);
        self.spots = ease(self.spots, spots_t, dt_sec, 420.0);
        self.hue = ease(self.hue, hue_t, dt_sec, 600.0);
        self.rhythm_t = ease(self.rhythm_t, sig.rhythm, dt_sec, 480.0);

        self.active_minutes += (active_sec as f64) / 60.0;
        if sig.focus > 0.45 {
            self.focus_minutes += (active_sec as f64) / 60.0 * sig.focus as f64;
        }

        let prev = self.stage;
        self.stage = stage_index(self.active_minutes);
        prev != self.stage
    }

    pub fn stage_name(&self) -> &'static str {
        STAGES[self.stage as usize].1
    }

    /// 当前阶段内的成长进度 0..1。
    pub fn growth(&self) -> f32 {
        let idx = self.stage as usize;
        let lo = STAGES[idx].0;
        let hi = if idx + 1 < STAGES.len() {
            STAGES[idx + 1].0
        } else {
            STAGES[idx].0 * 2.0
        };
        if hi <= lo {
            return 1.0;
        }
        clamp01(((self.active_minutes - lo) / (hi - lo)) as f32)
    }

    /// 一句话解读。
    pub fn caption(&self, sig: &Signature) -> &'static str {
        if sig.load > 0.62 {
            "它有点喘不过气，水也浑了。"
        } else if sig.rest_min > 95.0 {
            "它已经很久没歇过了。"
        } else if sig.active_ratio < 0.12 {
            "水里很安静，它在慢慢漂。"
        } else if sig.input_ratio < 0.12 && sig.active_ratio > 0.5 {
            "它安静地待着，像在思考。"
        } else if sig.calm < 0.32 {
            "切换太碎，它有点晕。"
        } else if sig.focus > 0.68 && sig.vitality_hint() {
            "它今天在稳定地发光。"
        } else if sig.variety > 0.72 {
            "它尝到了很多不同的味道。"
        } else {
            "它在自己找节奏。"
        }
    }
}

pub fn stage_index(minutes: f64) -> u8 {
    let mut idx = 0u8;
    for (i, (threshold, _)) in STAGES.iter().enumerate() {
        if minutes >= *threshold {
            idx = i as u8;
        }
    }
    idx
}

impl Signature {
    /// caption 的小辅助：平静度足够时才算“好状态”。
    fn vitality_hint(&self) -> bool {
        self.calm > 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(focus: f32, calm: f32, load: f32, activity: f32) -> Signature {
        Signature {
            focus,
            calm,
            load,
            active_ratio: activity,
            rhythm: 0.5,
            variety: 0.3,
            ..Signature::default()
        }
    }

    #[test]
    fn steady_focus_raises_glow() {
        let mut eco = Eco::default();
        let s = sig(0.9, 0.8, 0.0, 0.9);
        for _ in 0..600 {
            eco.tick(&s, 1.0, 1.0, 1_000_000);
        }
        assert!(eco.glow > 0.7, "glow={}", eco.glow);
        assert!(eco.vitality > 0.6, "vitality={}", eco.vitality);
    }

    #[test]
    fn overwork_clouds_water_and_spots() {
        let mut eco = Eco::default();
        let s = sig(0.5, 0.2, 0.9, 0.95);
        for _ in 0..1200 {
            eco.tick(&s, 1.0, 1.0, 1_000_000);
        }
        assert!(eco.turbidity > 0.55, "turbidity={}", eco.turbidity);
        assert!(eco.spots > 0.5, "spots={}", eco.spots);
        assert!(eco.vitality < 0.5, "vitality={}", eco.vitality);
    }

    #[test]
    fn offline_decay_keeps_progress() {
        let mut eco = Eco::default();
        eco.active_minutes = 200.0;
        eco.stage = stage_index(eco.active_minutes);
        eco.vitality = 0.9;
        eco.last_tick_ms = 1_000_000;
        eco.apply_offline(1_000_000 + 8 * 3600 * 1000);
        assert!(eco.vitality < 0.5, "vitality={}", eco.vitality);
        assert_eq!(eco.stage, 2);
        assert!(eco.active_minutes >= 200.0);
    }

    #[test]
    fn stages_advance() {
        assert_eq!(stage_index(0.0), 0);
        assert_eq!(stage_index(44.9), 0);
        assert_eq!(stage_index(45.0), 1);
        assert_eq!(stage_index(179.0), 1);
        assert_eq!(stage_index(180.0), 2);
        assert_eq!(stage_index(10_000.0), 3);
    }
}
