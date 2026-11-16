// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Which quality tier a device draws in, and the render path that draws it: by the device's capabilities, unless a
//! setting says otherwise, logged with the rule that chose it. See `design/rendering.md`.
pub use tomori_world::Tier;

/// A way of drawing the render world's scene, which each tier is drawn by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPath {
    /// Today's renderer: one pass for each eye, each material by the pipeline its shading model names, stylised.
    Stylised,
}

/// The path that draws `tier`: today's renderer for every tier, until High's and Ultra's own exist.
pub fn path(tier: Tier) -> RenderPath {
    match tier {
        Tier::Low | Tier::High | Tier::Ultra => RenderPath::Stylised,
    }
}

/// The headsets, by Android model, measured holding their frames in [`Tier::High`]: a Quest 3, as each eye's defaults
/// were measured on it.
pub const MEASURED: &[&str] = &["Quest 3"];

/// What a tier is chosen for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device<'a> {
    /// A headset, by its Android model, if it says one.
    Headset(Option<&'a str>),
    /// The desktop window, the spectator or `tomori --xr`, on a desktop GPU.
    Desktop,
}

/// A tier chosen, and the rule that chose it, which the log says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub tier: Tier,
    pub rule: String,
}

impl Choice {
    /// What the log says of it at startup.
    pub fn describe(&self) -> String {
        format!(
            "Render tier {}, {}; drawn by {:?}, as every tier is for now",
            self.tier,
            self.rule,
            path(self.tier)
        )
    }
}

/// The tier `device` draws in: as `setting` says, `low`, `high` or `ultra`, read from the setting `name`, such as the
/// headset's `tomori.tier` intent extra or `TOMORI_TIER`; or else by the device, High on a headset measured holding
/// its frames there, Low on any other, and Ultra on a desktop. A setting not understood keeps the device's with a
/// warning.
pub fn choose(device: Device<'_>, setting: Option<&str>, name: &str) -> Choice {
    let setting = setting.map(str::trim).filter(|value| !value.is_empty());
    match setting.map(|value| (value, Tier::parse(value))) {
        Some((_, Some(tier))) => {
            return Choice {
                tier,
                rule: format!("as {name} asks"),
            };
        }
        Some((value, None)) => {
            log::warn!("Ignored render tier {value:?} from {name}: use low, high or ultra");
        }
        None => {}
    }
    match device {
        Device::Headset(model) => {
            let model = model.map(str::trim);
            let named = model.unwrap_or("an unknown model");
            if model.is_some_and(|model| MEASURED.contains(&model)) {
                Choice {
                    tier: Tier::High,
                    rule: format!("by default for {named}, measured on this model"),
                }
            } else {
                Choice {
                    tier: Tier::Low,
                    rule: format!("by default for {named}, as it has not been measured in high"),
                }
            }
        }
        Device::Desktop => Choice {
            tier: Tier::Ultra,
            rule: "by default for a desktop GPU".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tier_is_drawn_by_today_s_renderer_for_now() {
        for tier in Tier::ALL {
            assert_eq!(path(tier), RenderPath::Stylised);
        }
    }

    #[test]
    fn a_device_draws_in_the_tier_its_capabilities_choose_unless_a_setting_says_otherwise() {
        log::set_max_level(log::LevelFilter::Trace);
        let quest = choose(Device::Headset(Some(" Quest 3 ")), None, "tomori.tier");
        assert_eq!(quest.tier, Tier::High);
        assert_eq!(
            quest.describe(),
            "Render tier high, by default for Quest 3, measured on this model; drawn by Stylised, as every tier is \
             for now"
        );
        for model in [Some("Quest 3S"), Some("Quest 2"), Some("Pico 4"), None] {
            let choice = choose(Device::Headset(model), Some(""), "tomori.tier");
            assert_eq!(choice.tier, Tier::Low, "{model:?}");
            assert!(choice.rule.contains("not been measured"), "{}", choice.rule);
        }
        assert!(
            choose(Device::Headset(None), None, "tomori.tier")
                .rule
                .contains("an unknown model")
        );
        let desktop = choose(Device::Desktop, None, "TOMORI_TIER");
        assert_eq!(
            (desktop.tier, desktop.rule.as_str()),
            (Tier::Ultra, "by default for a desktop GPU")
        );
        // A setting overrides it, and says so.
        let asked = choose(
            Device::Headset(Some("Quest 3")),
            Some(" Low "),
            "tomori.tier",
        );
        assert_eq!(
            asked,
            Choice {
                tier: Tier::Low,
                rule: "as tomori.tier asks".into()
            }
        );
        assert_eq!(
            choose(Device::Desktop, Some("high"), "TOMORI_TIER").tier,
            Tier::High
        );
        // One not understood keeps the device's.
        assert_eq!(
            choose(Device::Desktop, Some("medium"), "TOMORI_TIER").tier,
            Tier::Ultra
        );
    }
}
