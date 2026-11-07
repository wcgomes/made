//! OpenCode launch contribution: the composed configuration, passed to the
//! child under [`CONFIG_CONTENT_ENV`].

use made_bridge::config::CONFIG_CONTENT_ENV;

use crate::integrations::EnvContribution;

/// The composed configuration as a launch contribution, for the targets the
/// policy covers.
pub(crate) fn contribution(composed: Option<String>, covered: bool) -> EnvContribution {
    match composed {
        Some(content) if covered => EnvContribution {
            set: vec![(CONFIG_CONTENT_ENV.into(), content.into())],
            strip: Vec::new(),
        },
        _ => EnvContribution::default(),
    }
}

/// Whether `contribution` carries the composed configuration.
pub(crate) fn composes(contribution: &EnvContribution) -> bool {
    contribution
        .set
        .iter()
        .any(|(key, _)| key == CONFIG_CONTENT_ENV)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_covered_launch_receives_the_configuration() {
        let covered = contribution(Some("{}".to_owned()), true);
        assert!(composes(&covered));
        assert!(!composes(&contribution(Some("{}".to_owned()), false)));
        assert!(!composes(&contribution(None, true)));
    }
}
