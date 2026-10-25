//! 物种注册表与判定(设计见 docs/generation-collection.md §5.2)。
//!
//! 判定输入只有 Agg(终身聚合),纯函数:同一份聚合永远得到同一物种,
//! 可测试、可回放。阈值全部是常量,随调随改。

use crate::model::Agg;
use crate::signature::clamp01;

pub struct SpeciesDef {
    pub id: &'static str,
    pub name: &'static str,
    pub archetype: &'static str,
    /// 图鉴未解锁时的暗示文案:方向性描述,不透露阈值。
    pub hint: &'static str,
}

/// P0 六个常见种。顺序即判定优先顺序(平分时靠前的赢)。
pub const SPECIES: [SpeciesDef; 6] = [
    SpeciesDef {
        id: "deep_glow",
        name: "深光种",
        archetype: "medusa",
        hint: "长时间沉在专注里,缸底会亮起稳稳的光。",
    },
    SpeciesDef {
        id: "tide",
        name: "潮汐种",
        archetype: "medusa",
        hint: "工作和休息有来有回,像潮水一样守时。",
    },
    SpeciesDef {
        id: "school",
        name: "群游种",
        archetype: "school",
        hint: "同时张开许多条线,它便学会成群结队。",
    },
    SpeciesDef {
        id: "sunken_jade",
        name: "沉玉种",
        archetype: "star",
        hint: "在最浑的水里撑住,又自己清了回来。",
    },
    SpeciesDef {
        id: "dawn",
        name: "晨光种",
        archetype: "fish",
        hint: "趁上午把一天的大事做完,它会记住晨光。",
    },
    SpeciesDef {
        id: "night",
        name: "夜洄种",
        archetype: "fish",
        hint: "夜深了,它才游得最欢。",
    },
];

pub fn by_id(id: &str) -> Option<&'static SpeciesDef> {
    SPECIES.iter().find(|d| d.id == id)
}

/// 锁定阈值:最高分不足时保持未定型(继续按通用形态渲染)。
pub const LOCK_MIN: f32 = 0.55;
/// 时段判定需要的最少活跃小时数,样本太少不判晨型/夜型。
const PHASE_MIN_HOURS: f32 = 4.0;

fn min1(x: f32, cap: f32) -> f32 {
    if cap <= 0.0 {
        0.0
    } else {
        (x / cap).min(1.0)
    }
}

fn hour_share(agg: &Agg, pred: fn(u8) -> bool) -> (f32, f32) {
    let total: u32 = agg.hour_hist.iter().sum();
    let part: u32 = agg
        .hour_hist
        .iter()
        .enumerate()
        .filter(|(h, _)| pred(*h as u8))
        .map(|(_, v)| v)
        .sum();
    (part as f32 / total.max(1) as f32, total as f32 / 3600.0)
}

fn is_morning(h: u8) -> bool {
    (6..=11).contains(&h)
}

fn is_night(h: u8) -> bool {
    h >= 21 || h <= 5
}

/// 各物种匹配分 0..1,顺序同 SPECIES。
pub fn scores(agg: &Agg) -> [f32; 6] {
    let deep_glow = clamp01(
        0.70 * agg.focus_ema + 0.45 * min1(agg.deep_focus_min as f32, 180.0) + 0.15 * agg.calm_ema,
    );
    let tide = clamp01(0.80 * agg.rhythm_ema + 0.30 * agg.calm_ema);
    let school = clamp01(0.95 * agg.variety_ema * min1(agg.variety_seen_sec as f32, 6.0 * 3600.0));
    // 沉玉种要先经历过高负荷,并且"回来了":当前负荷低说明已经缓过来,
    // 还在水深火热里的不算。
    let sunken_jade = if agg.overwork_days == 0 {
        0.1
    } else {
        clamp01(0.15 + 0.35 * min1(agg.overwork_days as f32, 3.0) + 0.55 * (1.0 - agg.load_ema))
    };
    let (m_share, total_h) = hour_share(agg, is_morning);
    let dawn = if total_h < PHASE_MIN_HOURS {
        0.1
    } else {
        clamp01(1.7 * m_share.powi(2))
    };
    let (n_share, _) = hour_share(agg, is_night);
    let night = if total_h < PHASE_MIN_HOURS {
        0.1
    } else {
        clamp01(1.7 * n_share.powi(2))
    };
    [deep_glow, tide, school, sunken_jade, dawn, night]
}

fn argmax(scores: &[f32; 6]) -> usize {
    let mut best = 0;
    for (i, s) in scores.iter().enumerate() {
        if *s > scores[best] {
            best = i;
        }
    }
    best
}

/// 成体时尝试判定;None = 尚未定型。
pub fn determine(agg: &Agg) -> Option<usize> {
    let s = scores(agg);
    let i = argmax(&s);
    (s[i] >= LOCK_MIN).then_some(i)
}

/// 繁盛时的兜底:收集必须有物种,取最高分。
pub fn best(agg: &Agg) -> usize {
    argmax(&scores(agg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agg() -> Agg {
        Agg::neutral()
    }

    #[test]
    fn neutral_agg_stays_undetermined() {
        assert!(determine(&agg()).is_none());
        // 兜底永远有结果
        assert!(best(&agg()) < SPECIES.len());
    }

    #[test]
    fn deep_focus_becomes_deep_glow() {
        let mut a = agg();
        a.focus_ema = 0.8;
        a.calm_ema = 0.75;
        a.deep_focus_min = 200.0;
        assert_eq!(determine(&a), Some(0));
    }

    #[test]
    fn steady_rhythm_becomes_tide() {
        let mut a = agg();
        a.rhythm_ema = 0.85;
        a.calm_ema = 0.7;
        assert_eq!(determine(&a), Some(1));
    }

    #[test]
    fn high_variety_becomes_school() {
        let mut a = agg();
        a.variety_ema = 0.8;
        a.variety_seen_sec = 7 * 3600;
        assert_eq!(determine(&a), Some(2));
    }

    #[test]
    fn survived_crunch_becomes_sunken_jade() {
        let mut a = agg();
        a.overwork_days = 2;
        a.load_ema = 0.15; // 已经缓过来
        assert_eq!(determine(&a), Some(3));
    }

    #[test]
    fn unrecovered_overwork_is_not_jade() {
        let mut a = agg();
        a.overwork_days = 2;
        a.load_ema = 0.9; // 还在水深火热里
        assert_ne!(determine(&a), Some(3));
    }

    #[test]
    fn morning_person_becomes_dawn() {
        let mut a = agg();
        for h in 6..=11 {
            a.hour_hist[h] = 3600;
        }
        for h in 13..=18 {
            a.hour_hist[h] = 1800;
        }
        assert_eq!(determine(&a), Some(4));
    }

    #[test]
    fn night_owl_becomes_night() {
        let mut a = agg();
        for h in [22u8, 23, 0, 1] {
            a.hour_hist[h as usize] = 3600;
        }
        assert_eq!(determine(&a), Some(5));
    }

    #[test]
    fn thin_time_sample_is_never_phase_species() {
        let mut a = agg();
        a.hour_hist[8] = 1800; // 只有 30 分钟样本
        assert_ne!(determine(&a), Some(4));
        assert_ne!(determine(&a), Some(5));
    }
}
