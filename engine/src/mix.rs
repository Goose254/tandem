//! Channel mixing between the source's layout and an output's.
//!
//! Layouts are assumed from the channel count (the standard Windows orders). A channel the
//! output lacks is folded into its nearest speakers instead of being dropped: a 5.1 film on
//! stereo speakers keeps its dialogue (centre) and its surrounds.

/// -3 dB: keeps the power of a channel that is split across two speakers.
const HALF_POWER: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    FrontLeft,
    FrontRight,
    Centre,
    Lfe,
    BackLeft,
    BackRight,
    SideLeft,
    SideRight,
    /// No known position (beyond 7.1, or the back centre of 6.1).
    Unknown,
}

use Role::*;

fn layout(channels: usize) -> Vec<Role> {
    let known: &[Role] = match channels {
        1 => &[Centre],
        2 => &[FrontLeft, FrontRight],
        3 => &[FrontLeft, FrontRight, Lfe],
        4 => &[FrontLeft, FrontRight, BackLeft, BackRight],
        5 => &[FrontLeft, FrontRight, Centre, BackLeft, BackRight],
        6 | 7 => &[FrontLeft, FrontRight, Centre, Lfe, BackLeft, BackRight],
        _ => &[
            FrontLeft, FrontRight, Centre, Lfe, BackLeft, BackRight, SideLeft, SideRight,
        ],
    };
    let mut roles = known.to_vec();
    roles.resize(channels, Unknown);
    roles
}

/// Where a source channel goes when the output has no speaker for it: `(role, gain)` choices
/// in order of preference; the first whose roles all exist wins.
fn fallbacks(role: Role) -> &'static [&'static [(Role, f32)]] {
    match role {
        Centre => &[&[(FrontLeft, HALF_POWER), (FrontRight, HALF_POWER)]],
        BackLeft => &[&[(SideLeft, 1.0)], &[(FrontLeft, HALF_POWER)]],
        BackRight => &[&[(SideRight, 1.0)], &[(FrontRight, HALF_POWER)]],
        SideLeft => &[&[(BackLeft, 1.0)], &[(FrontLeft, HALF_POWER)]],
        SideRight => &[&[(BackRight, 1.0)], &[(FrontRight, HALF_POWER)]],
        // Bass effects are optional by design, and summing them into small speakers clips.
        FrontLeft | FrontRight | Lfe | Unknown => &[],
    }
}

/// The gain of every source channel in every output channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    /// For each output channel: the source channels it plays, with their gains.
    rows: Vec<Vec<(usize, f32)>>,
}

impl Matrix {
    pub fn new(src_channels: usize, dst_channels: usize) -> Self {
        let src = layout(src_channels);
        let dst = layout(dst_channels);
        let mut rows = vec![Vec::new(); dst_channels];
        let position = |role: Role| dst.iter().position(|r| *r == role && role != Unknown);

        for (s, &role) in src.iter().enumerate() {
            if src_channels == dst_channels {
                rows[s].push((s, 1.0));
            } else if src_channels == 1 {
                // Mono plays at full level on both front speakers, as Windows does.
                match (position(FrontLeft), position(FrontRight)) {
                    (Some(l), Some(r)) => {
                        rows[l].push((s, 1.0));
                        rows[r].push((s, 1.0));
                    }
                    _ => rows[0].push((s, 1.0)),
                }
            } else if dst_channels == 1 {
                // Everything but bass effects, averaged over the left/right pair it belongs to.
                let gain = match role {
                    FrontLeft | FrontRight => 0.5,
                    BackLeft | BackRight | SideLeft | SideRight => 0.5 * HALF_POWER,
                    Centre => HALF_POWER,
                    Lfe | Unknown => 0.0,
                };
                if gain > 0.0 {
                    rows[0].push((s, gain));
                }
            } else if let Some(d) = position(role) {
                rows[d].push((s, 1.0));
            } else if let Some(choice) = fallbacks(role)
                .iter()
                .find(|choice| choice.iter().all(|(r, _)| position(*r).is_some()))
            {
                for (r, gain) in choice.iter() {
                    rows[position(*r).expect("checked")].push((s, *gain));
                }
            }
        }
        Self { rows }
    }

    /// One output sample: `frame` holds one source frame.
    #[inline]
    pub fn mix(&self, frame: &[f32], out_channel: usize) -> f32 {
        self.rows[out_channel]
            .iter()
            .map(|&(s, g)| frame[s] * g)
            .sum()
    }

    #[cfg(test)]
    fn gain(&self, src: usize, dst: usize) -> f32 {
        self.rows[dst]
            .iter()
            .filter(|(s, _)| *s == src)
            .map(|(_, g)| g)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_layout_is_identity() {
        for n in [1, 2, 6, 8] {
            let m = Matrix::new(n, n);
            for s in 0..n {
                for d in 0..n {
                    assert_eq!(m.gain(s, d), if s == d { 1.0 } else { 0.0 });
                }
            }
        }
    }

    #[test]
    fn mono_goes_to_both_fronts_only() {
        let m = Matrix::new(1, 6);
        assert_eq!((m.gain(0, 0), m.gain(0, 1)), (1.0, 1.0));
        assert!((2..6).all(|d| m.gain(0, d) == 0.0));
    }

    #[test]
    fn stereo_to_mono_averages_both_sides() {
        let m = Matrix::new(2, 1);
        assert_eq!((m.gain(0, 0), m.gain(1, 0)), (0.5, 0.5));
    }

    #[test]
    fn surround_folds_into_stereo() {
        let m = Matrix::new(6, 2);
        // FL, FR, C, LFE, BL, BR
        assert_eq!((m.gain(0, 0), m.gain(0, 1)), (1.0, 0.0));
        assert_eq!((m.gain(2, 0), m.gain(2, 1)), (HALF_POWER, HALF_POWER));
        assert_eq!((m.gain(3, 0), m.gain(3, 1)), (0.0, 0.0));
        assert_eq!((m.gain(4, 0), m.gain(4, 1)), (HALF_POWER, 0.0));
        assert_eq!((m.gain(5, 0), m.gain(5, 1)), (0.0, HALF_POWER));
        let m = Matrix::new(8, 2);
        assert_eq!((m.gain(6, 0), m.gain(7, 1)), (HALF_POWER, HALF_POWER));
    }

    #[test]
    fn seven_one_sides_move_to_five_one_backs() {
        let m = Matrix::new(8, 6);
        assert_eq!((m.gain(4, 4), m.gain(6, 4)), (1.0, 1.0));
        assert_eq!((m.gain(5, 5), m.gain(7, 5)), (1.0, 1.0));
    }

    #[test]
    fn stereo_stays_on_the_fronts_of_a_surround_output() {
        let m = Matrix::new(2, 8);
        assert_eq!((m.gain(0, 0), m.gain(1, 1)), (1.0, 1.0));
        assert!((2..8).all(|d| m.gain(0, d) == 0.0 && m.gain(1, d) == 0.0));
    }
}
