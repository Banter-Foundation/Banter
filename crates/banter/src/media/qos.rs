//! Sender congestion ladder. The step order lives in `banter_server` so Linux
//! and Windows share one state machine and one offer shape.

use banter_server::{CongestionLadder, LadderStep, LadderTargets, LayerId, MediaKind};

/// One stats sample. Loss is a fraction, `available_ratio` is estimated bitrate over the cap.
pub fn sample_is_congested(loss: f32, rtt_ms: u32, available_ratio: f32) -> bool {
    loss >= 0.04 || rtt_ms >= 250 || (available_ratio > 0.0 && available_ratio < 0.70)
}

#[derive(Clone, Debug)]
pub struct SenderQos {
    ladder: CongestionLadder,
    loss: f32,
    rtt_ms: u32,
    jitter_ms: u32,
}

impl SenderQos {
    pub fn new() -> Self {
        Self {
            ladder: CongestionLadder::new(),
            loss: 0.0,
            rtt_ms: 0,
            jitter_ms: 0,
        }
    }

    pub fn loss(&self) -> f32 {
        self.loss
    }

    pub fn rtt_ms(&self) -> u32 {
        self.rtt_ms
    }

    pub fn jitter_ms(&self) -> u32 {
        self.jitter_ms
    }

    pub fn step(&self) -> LadderStep {
        self.ladder.step()
    }

    pub fn targets(&self) -> LadderTargets {
        self.ladder.targets()
    }

    pub fn observe(&mut self, loss: f32, rtt_ms: u32, available_ratio: f32) -> LadderTargets {
        self.jitter_ms = self.rtt_ms.abs_diff(rtt_ms);
        self.loss = loss;
        self.rtt_ms = rtt_ms;
        self.ladder
            .note(sample_is_congested(loss, rtt_ms, available_ratio));
        self.ladder.targets()
    }

    /// Clear a stuck ScreenStop so a new Go Live is not held down by an old congestion climb.
    pub fn reset_ladder(&mut self) {
        self.ladder = CongestionLadder::new();
        self.loss = 0.0;
        self.rtt_ms = 0;
        self.jitter_ms = 0;
    }

    /// Layers to encode: every requested layer at or below the ladder and the peer cap.
    pub fn active_layers(
        &self,
        track: MediaKind,
        wanted: Option<&[LayerId]>,
        peer_cap: LayerId,
    ) -> Vec<LayerId> {
        let sender_max = match track {
            MediaKind::Camera => self.targets().camera,
            MediaKind::Screen => {
                if !self.targets().screen_published {
                    return Vec::new();
                }
                self.targets().screen
            }
            _ => return Vec::new(),
        };
        // When dynacast has not spoken yet, encode only the peer-cap layer (Full by
        // default). Encoding q+h+f until WantedLayers arrives burned ~1.3× uplink and
        // 2–3× encode work. P2P never gets WantedLayers; peer_cap (and its force_key)
        // is the subscribe signal there.
        let published: Vec<LayerId> = match wanted {
            Some(layers) => layers
                .iter()
                .copied()
                .filter(|layer| *layer != LayerId::Off)
                .collect(),
            None => match peer_cap {
                LayerId::Off => Vec::new(),
                layer => vec![layer],
            },
        };
        let mut chosen = Vec::new();
        for layer in published {
            if layer.rank() > 0
                && layer.rank() <= sender_max.rank()
                && layer.rank() <= peer_cap.rank()
            {
                chosen.push(layer);
            }
        }
        chosen.sort_by_key(|layer| layer.rank());
        chosen.dedup();
        chosen
    }

    /// Highest layer that the ladder, the receiver, and dynacast all still want.
    #[allow(dead_code)]
    pub fn active_layer(
        &self,
        track: MediaKind,
        wanted: Option<&[LayerId]>,
        peer_cap: LayerId,
    ) -> LayerId {
        self.active_layers(track, wanted, peer_cap)
            .into_iter()
            .max_by_key(|layer| layer.rank())
            .unwrap_or(LayerId::Off)
    }
}

impl Default for SenderQos {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use banter_server::{LADDER_DOWN_TICKS, LADDER_UP_TICKS};

    #[test]
    fn congestion_follows_the_ladder_and_spares_user_audio() {
        let mut qos = SenderQos::new();
        let order = [
            LadderStep::CameraHalf,
            LadderStep::StreamAudioHalf,
            LadderStep::ScreenHalf,
            LadderStep::CameraCut,
            LadderStep::StreamAudioCut,
            LadderStep::ScreenStop,
        ];
        for expected in order {
            for _ in 0..LADDER_DOWN_TICKS {
                qos.observe(0.2, 40, 1.0);
            }
            assert_eq!(qos.step(), expected);
            assert_eq!(qos.targets().user_audio, 1.0);
        }
        for _ in 0..LADDER_DOWN_TICKS * 4 {
            qos.observe(0.5, 400, 0.1);
        }
        assert_eq!(qos.step(), LadderStep::ScreenStop);
        assert_eq!(qos.targets().user_audio, 1.0);
        assert!(!qos.targets().screen_published);
    }

    #[test]
    fn recovery_takes_more_clear_samples_than_a_step_down() {
        let mut qos = SenderQos::new();
        for _ in 0..LADDER_DOWN_TICKS {
            qos.observe(0.1, 10, 1.0);
        }
        assert_eq!(qos.step(), LadderStep::CameraHalf);
        for _ in 0..LADDER_UP_TICKS - 1 {
            qos.observe(0.0, 20, 1.0);
        }
        assert_eq!(qos.step(), LadderStep::CameraHalf);
        qos.observe(0.0, 20, 1.0);
        assert_eq!(qos.step(), LadderStep::Nominal);
    }

    #[test]
    fn dynacast_and_the_slider_cannot_outrank_the_ladder() {
        let mut qos = SenderQos::new();
        for _ in 0..LADDER_DOWN_TICKS {
            qos.observe(0.2, 10, 1.0);
        }
        let layer = qos.active_layer(
            MediaKind::Camera,
            Some(&[LayerId::Full, LayerId::Half]),
            LayerId::Full,
        );
        assert_eq!(layer, LayerId::Half);
        assert_eq!(
            qos.active_layer(MediaKind::Camera, Some(&[]), LayerId::Full),
            LayerId::Off
        );
    }

    #[test]
    fn reset_ladder_restores_screen_publish_after_screen_stop() {
        let mut qos = SenderQos::new();
        for _ in 0..LADDER_DOWN_TICKS * 8 {
            qos.observe(0.5, 400, 0.1);
        }
        assert_eq!(qos.step(), LadderStep::ScreenStop);
        assert!(!qos.targets().screen_published);
        qos.reset_ladder();
        assert_eq!(qos.step(), LadderStep::Nominal);
        assert!(qos.targets().screen_published);
        assert!(!qos
            .active_layers(MediaKind::Screen, None, LayerId::Full)
            .is_empty());
    }

    #[test]
    fn unused_layers_stay_idle_until_dynacast_or_peer_cap() {
        let qos = SenderQos::new();
        assert_eq!(
            qos.active_layers(MediaKind::Screen, None, LayerId::Full),
            vec![LayerId::Full]
        );
        assert_eq!(
            qos.active_layers(MediaKind::Camera, None, LayerId::Half),
            vec![LayerId::Half]
        );
        assert_eq!(
            qos.active_layers(
                MediaKind::Screen,
                Some(&[LayerId::Full, LayerId::Half, LayerId::Quarter]),
                LayerId::Full
            ),
            vec![LayerId::Quarter, LayerId::Half, LayerId::Full]
        );
    }
}
