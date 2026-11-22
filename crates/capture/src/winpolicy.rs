//! Windows backend choice and the DXGI failure policy (spec 02). Pure, so it is tested on
//! every target.

use arrowhead_core::config::WindowsBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dxgi,
    WgcMonitor,
    WgcWindow,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Dxgi => "dxgi",
            Kind::WgcMonitor => "wgc-monitor",
            Kind::WgcWindow => "wgc-window",
        }
    }
}

/// Backends to try in order for the `capture.windows_backend` setting. Only `auto` falls
/// back; a named backend that fails is an error the user should see.
pub fn backend_order(b: WindowsBackend) -> &'static [Kind] {
    match b {
        WindowsBackend::Auto => &[Kind::Dxgi, Kind::WgcMonitor, Kind::WgcWindow],
        WindowsBackend::Dxgi => &[Kind::Dxgi],
        WindowsBackend::WgcMonitor => &[Kind::WgcMonitor],
        WindowsBackend::WgcWindow => &[Kind::WgcWindow],
    }
}

/// "`limit` failures within `span_ms`" (DXGI: 3 in 5 s).
pub struct FailureWindow {
    times: Vec<u64>,
    limit: usize,
    span_ms: u64,
}

impl FailureWindow {
    pub fn new(limit: usize, span_ms: u64) -> Self {
        Self {
            times: Vec::new(),
            limit,
            span_ms,
        }
    }

    /// Records a failure at `now_ms`; true once `limit` failures fall inside the window.
    pub fn record(&mut self, now_ms: u64) -> bool {
        let span = self.span_ms;
        self.times.retain(|&t| now_ms.saturating_sub(t) < span);
        self.times.push(now_ms);
        self.times.len() >= self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_falls_back_named_does_not() {
        assert_eq!(
            backend_order(WindowsBackend::Auto),
            [Kind::Dxgi, Kind::WgcMonitor, Kind::WgcWindow]
        );
        assert_eq!(backend_order(WindowsBackend::Dxgi), [Kind::Dxgi]);
        assert_eq!(backend_order(WindowsBackend::WgcWindow), [Kind::WgcWindow]);
    }

    #[test]
    fn three_in_five_seconds_trips() {
        let mut w = FailureWindow::new(3, 5000);
        assert!(!w.record(0));
        assert!(!w.record(2000));
        assert!(w.record(4999));
    }

    #[test]
    fn spread_out_failures_do_not_trip() {
        let mut w = FailureWindow::new(3, 5000);
        assert!(!w.record(0));
        assert!(!w.record(3000));
        // The first one is 5000 ms old by now and drops out.
        assert!(!w.record(5000));
        assert!(!w.record(9000));
        assert!(w.record(9500));
    }
}
