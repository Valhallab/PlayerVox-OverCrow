use overcrow_logging::EventLogger;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Provider {
    Mpris,
    WarframeWorldstate,
    WarframeMarket,
}

impl Provider {
    const fn fields(self) -> &'static str {
        match self {
            Self::Mpris => "widget=media provider=mpris",
            Self::WarframeWorldstate => {
                "provider=warframe_worldstate affected_widgets=warframe_status,warframe_fissures,warframe_sortie,warframe_invasions"
            }
            Self::WarframeMarket => "widget=warframe_market provider=warframe_market",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureCategory {
    Startup,
    Connection,
    Discovery,
    Command,
    Timeout,
    Transport,
    Http,
    Parse,
    Validation,
    Response,
}

impl FailureCategory {
    const fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Connection => "connection",
            Self::Discovery => "discovery",
            Self::Command => "command",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::Http => "http",
            Self::Parse => "parse",
            Self::Validation => "validation",
            Self::Response => "response",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transition {
    Failed(FailureCategory),
    Recovered,
}

#[derive(Default)]
struct HealthTracker {
    failure: Option<FailureCategory>,
}

impl HealthTracker {
    fn failed(&mut self, category: FailureCategory) -> Option<Transition> {
        if self.failure == Some(category) {
            return None;
        }
        self.failure = Some(category);
        Some(Transition::Failed(category))
    }

    fn recovered(&mut self) -> Option<Transition> {
        self.failure.take().map(|_| Transition::Recovered)
    }
}

pub(crate) struct ProviderDiagnostics {
    logger: EventLogger,
    provider: Provider,
    tracker: HealthTracker,
}

impl ProviderDiagnostics {
    pub(crate) fn new(logger: EventLogger, provider: Provider) -> Self {
        Self {
            logger,
            provider,
            tracker: HealthTracker::default(),
        }
    }

    pub(crate) fn failed(&mut self, category: FailureCategory) {
        if let Some(Transition::Failed(category)) = self.tracker.failed(category) {
            self.logger.warn(
                "widget_provider_failed",
                format_args!("{} category={}", self.provider.fields(), category.name()),
            );
        }
    }

    pub(crate) fn recovered(&mut self) {
        if self.tracker.recovered().is_some() {
            self.logger.info(
                "widget_provider_recovered",
                format_args!("{}", self.provider.fields()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use overcrow_logging::EventLogger;

    use super::{FailureCategory, HealthTracker, Provider, ProviderDiagnostics, Transition};

    #[test]
    fn repeated_failures_are_suppressed_until_category_changes_or_provider_recovers() {
        let mut tracker = HealthTracker::default();

        assert_eq!(
            tracker.failed(FailureCategory::Connection),
            Some(Transition::Failed(FailureCategory::Connection))
        );
        assert_eq!(tracker.failed(FailureCategory::Connection), None);
        assert_eq!(
            tracker.failed(FailureCategory::Discovery),
            Some(Transition::Failed(FailureCategory::Discovery))
        );
        assert_eq!(tracker.recovered(), Some(Transition::Recovered));
        assert_eq!(tracker.recovered(), None);
    }

    #[test]
    fn provider_fields_are_fixed_and_contain_no_runtime_text() {
        assert_eq!(Provider::Mpris.fields(), "widget=media provider=mpris");
        assert_eq!(
            Provider::WarframeWorldstate.fields(),
            "provider=warframe_worldstate affected_widgets=warframe_status,warframe_fissures,warframe_sortie,warframe_invasions"
        );
        assert_eq!(
            Provider::WarframeMarket.fields(),
            "widget=warframe_market provider=warframe_market"
        );
    }

    #[test]
    fn diagnostics_accept_every_stable_category_without_runtime_text() {
        let mut diagnostics = ProviderDiagnostics::new(EventLogger::disabled(), Provider::Mpris);

        for category in [
            FailureCategory::Startup,
            FailureCategory::Connection,
            FailureCategory::Discovery,
            FailureCategory::Command,
            FailureCategory::Timeout,
            FailureCategory::Transport,
            FailureCategory::Http,
            FailureCategory::Parse,
            FailureCategory::Validation,
            FailureCategory::Response,
        ] {
            diagnostics.failed(category);
        }
        diagnostics.recovered();
    }
}
