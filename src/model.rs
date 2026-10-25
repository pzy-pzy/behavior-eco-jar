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

/// 个体基因:同物种内的随机差异,出生时定、终身不变,收集后随档案冻存。
/// 默认值即旧版的视觉常数,保证无基因档案时外观不变。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Genome {
    pub seed: u32,
    /// blob 形变的三次谐波权重。
    pub wob: [f32; 3],
    pub cilia_bias: i8,
    pub size: f32,
    pub hue_bias: f32,
    pub vacuoles: u8,
    pub satellites_bias: i8,
}

impl Genome {
    /// 由种子确定性生成:同一颗孢子在任何时候读档都长同一个样子。
    /// 谐波/体型等在旧版视觉常数(0.55/0.32/0.20、1.0……)的 ±20% 内浮动。
    pub fn from_seed(seed: u32) -> Self {
        let mut s = seed.wrapping_mul(2654435761) | 1;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as f32 / u32::MAX as f32
        };
        let wob = [
            0.55 * (0.8 + rnd() * 0.4),
            0.32 * (0.8 + rnd() * 0.4),
            0.20 * (0.8 + rnd() * 0.4),
        ];
        Self {
            seed,
            wob,
            cilia_bias: (rnd() * 5.0) as i8 - 2,
            size: 0.92 + rnd() * 0.16,
            hue_bias: rnd() * 0.12 - 0.06,
            vacuoles: 2 + (rnd() * 3.0) as u8,
            satellites_bias: (rnd() * 3.0) as i8 - 1,
        }
    }

    pub fn random() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_nanos() & 0xFFFF_FFFF) as u32)
            .unwrap_or(20261022);
        Self::from_seed(seed | 1)
    }
}

/// 一代生物的终身行为聚合,物种判定的唯一输入(见 docs/generation-collection.md §5.2)。
/// 只在活跃秒更新:挂机时 signature 反映的是"休息",不该稀释工作风格。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agg {
    pub focus_ema: f32,
    pub calm_ema: f32,
    pub rhythm_ema: f32,
    pub load_ema: f32,
    pub variety_ema: f32,
    /// variety 可测(存在过多个前台窗口)的活跃秒数。
    pub variety_seen_sec: u64,
    /// 各本地小时的活跃秒数直方图,判定晨型/夜型。
    pub hour_hist: [u32; 24],
    /// 深专注(focus 超阈值)累计分钟数。
    pub deep_focus_min: f64,
    /// 当日滚动累计,跨天时结算进 overwork_days。
    pub day_load_sum: f32,
    pub day_active_sec: u32,
    pub overwork_days: u32,
    /// 这一代覆盖的自然日数。
    pub days: u32,
}

/// 签名 EMA 的时间常数:4 小时前的行为对均值还剩 1/e 的权重。
pub const EMA_TAU_SEC: f32 = 4.0 * 3600.0;
pub const DEEP_FOCUS: f32 = 0.65;
/// 单日负荷均值超过该值且活跃不少于 2 小时,记一个高负荷日。
pub const OVERWORK_DAY_LOAD: f32 = 0.45;
pub const OVERWORK_DAY_MIN_SEC: u32 = 2 * 3600;

impl Agg {
    pub fn neutral() -> Self {
        Self {
            focus_ema: 0.25,
            calm_ema: 0.6,
            rhythm_ema: 0.35,
            load_ema: 0.1,
            variety_ema: 0.3,
            variety_seen_sec: 0,
            hour_hist: [0; 24],
            deep_focus_min: 0.0,
            day_load_sum: 0.0,
            day_active_sec: 0,
            overwork_days: 0,
            days: 1,
        }
    }

    /// 旧档迁移:没有逐秒历史,只用 eco 里仅有的两个累计量粗估,图鉴中如实标注"数据不全"。
    pub fn estimated(eco: &Eco) -> Self {
        let mut a = Self::neutral();
        if eco.active_minutes > 1.0 {
            a.focus_ema = clamp01((eco.focus_minutes / eco.active_minutes) as f32);
            a.days = ((eco.active_minutes / 480.0).ceil() as u32).max(1);
        }
        a
    }

    /// 每活跃秒推进一次。
    pub fn tick(&mut self, sig: &Signature, hour: u8) {
        let k = 1.0 - (-1.0 / EMA_TAU_SEC).exp();
        self.focus_ema += (sig.focus - self.focus_ema) * k;
        self.calm_ema += (sig.calm - self.calm_ema) * k;
        self.rhythm_ema += (sig.rhythm - self.rhythm_ema) * k;
        self.load_ema += (sig.load - self.load_ema) * k;
        if sig.variety_known {
            self.variety_ema += (sig.variety - self.variety_ema) * k;
            self.variety_seen_sec += 1;
        }
        self.hour_hist[(hour as usize).min(23)] += 1;
        if sig.focus > DEEP_FOCUS {
            self.deep_focus_min += 1.0 / 60.0;
        }
        self.day_load_sum += sig.load;
        self.day_active_sec += 1;
    }

    /// 跨天结算。
    pub fn roll_day(&mut self) {
        if self.day_active_sec >= OVERWORK_DAY_MIN_SEC {
            let avg = self.day_load_sum / self.day_active_sec as f32;
            if avg > OVERWORK_DAY_LOAD {
                self.overwork_days += 1;
            }
        }
        self.day_load_sum = 0.0;
        self.day_active_sec = 0;
        self.days += 1;
    }
}

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

    #[test]
    fn genome_from_seed_is_deterministic_and_sane() {
        let a = Genome::from_seed(12345);
        let b = Genome::from_seed(12345);
        assert_eq!(a.wob, b.wob);
        assert_eq!(a.seed, 12345);
        assert!((0.9..=1.1).contains(&a.size));
        assert!(a.hue_bias.abs() <= 0.06 + 1e-6);
        assert!((2..=4).contains(&a.vacuoles));
        assert!((-2..=2).contains(&a.cilia_bias));
        assert!((-1..=1).contains(&a.satellites_bias));
        assert_ne!(Genome::from_seed(1).wob, Genome::from_seed(2).wob);
    }

    #[test]
    fn agg_accumulates_and_rolls_days() {
        let mut a = Agg::neutral();
        let sig = Signature {
            focus: 0.9,
            calm: 0.8,
            rhythm: 0.6,
            load: 0.2,
            variety: 0.4,
            variety_known: true,
            ..Signature::default()
        };
        for _ in 0..3600 {
            a.tick(&sig, 10);
        }
        // 4 小时常数下一小时高强度专注:ema ≈ 0.25 + 0.65 * (1 - e^-0.25) ≈ 0.39
        assert!(a.focus_ema > 0.3 && a.focus_ema < 0.9, "focus_ema={}", a.focus_ema);
        assert_eq!(a.hour_hist[10], 3600);
        assert!((a.deep_focus_min - 60.0).abs() < 0.01);
        assert_eq!(a.variety_seen_sec, 3600);

        // 高负荷半天后跨天:应结算出一个高负荷日。
        let heavy = Signature {
            load: 0.8,
            focus: 0.5,
            calm: 0.3,
            ..sig
        };
        for _ in 0..(3 * 3600) {
            a.tick(&heavy, 11);
        }
        a.roll_day();
        assert_eq!(a.overwork_days, 1);
        assert_eq!(a.days, 2);
        assert_eq!(a.day_active_sec, 0);
    }

    #[test]
    fn agg_ignores_unknown_variety() {
        let mut a = Agg::neutral();
        let sig = Signature {
            variety: 0.9,
            variety_known: false,
            ..Signature::default()
        };
        a.tick(&sig, 0);
        assert_eq!(a.variety_seen_sec, 0);
        assert!((a.variety_ema - 0.3).abs() < 0.001);
    }
}
