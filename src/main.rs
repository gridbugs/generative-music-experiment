use currawong::prelude::*;
use rand::{rngs::StdRng, Rng, SeedableRng};

const C_MAJOR_SCALE: &[NoteName] = &[
    NoteName::A,
    NoteName::B,
    NoteName::C,
    NoteName::D,
    NoteName::E,
    NoteName::F,
    NoteName::G,
];

fn make_scale_base_freqs(note_names: &[NoteName]) -> Vec<Sfreq> {
    note_names
        .into_iter()
        .map(|&name| const_(Note { name, octave: 0 }.freq()))
        .collect()
}

fn random_note_c_major(base_hz: Sf64, range_hz: Sf64) -> Sfreq {
    sfreq_hz(base_hz + (noise_01() * range_hz))
        .filter(quantize_to_scale(make_scale_base_freqs(C_MAJOR_SCALE)).build())
}

fn voice(freq: Sfreq, gate: Gate) -> Sf64 {
    let freq_hz = freq.hz();
    let osc = oscillator_hz(Waveform::Saw, freq_hz.clone()).build()
        + oscillator_hz(Waveform::Saw, freq_hz.clone() * 2.0).build();
    let env_amp = adsr_linear_01(&gate).attack_s(0.01).release_s(2.0).build();
    let env_lpf = adsr_linear_01(&gate)
        .attack_s(0.01)
        .release_s(0.3)
        .build()
        .exp_01(1.0);
    osc.filter(low_pass_chebyshev(8000.0 * env_lpf).resonance(4.0).build())
        .filter(saturate().scale(2.0).min(-1.0).max(1.0).build())
        .mul_lazy(&env_amp)
}

fn random_replace_loop(
    trigger: Trigger,
    anchor: Sfreq,
    palette: Sfreq,
    length: usize,
    replace_probability_01: Sf64,
    anchor_probability_01: Sf64,
) -> Sfreq {
    let mut rng = StdRng::from_entropy();
    let mut sequence: Vec<Option<Freq>> = vec![None; length];
    let mut index = 0;
    let mut anchor_on_0 = false;
    let mut first_note = true;
    Signal::from_fn_mut(move |ctx| {
        let trigger = trigger.sample(ctx);
        if trigger {
            if rng.gen::<f64>() < replace_probability_01.sample(ctx) {
                sequence[index] = Some(palette.sample(ctx));
            }
            if index == 0 {
                anchor_on_0 = rng.gen::<f64>() < anchor_probability_01.sample(ctx);
            } else {
                first_note = false;
            }
        }
        let freq = if first_note {
            anchor.sample(ctx)
        } else if anchor_on_0 && index == 0 {
            anchor.sample(ctx)
        } else if let Some(freq) = sequence[index] {
            freq
        } else {
            let freq = palette.sample(ctx);
            sequence[index] = Some(freq);
            freq
        };
        if trigger {
            index = (index + 1) % sequence.len();
        }
        freq
    })
}

fn synth_signal(trigger: Trigger) -> Sf64 {
    let trigger = trigger.random_skip(0.1);
    let freq = random_replace_loop(
        trigger.clone(),
        const_(
            Note {
                name: NoteName::A,
                octave: 1,
            }
            .freq(),
        ),
        random_note_c_major(const_(50.0), const_(200.0)),
        8,
        const_(0.1),
        const_(0.5),
    );
    let gate = trigger.to_gate_with_duration_s(0.02);
    let modulate = 1.0
        - oscillator_s(Waveform::Triangle, 60.0)
            .build()
            .signed_to_01();
    let lfo = oscillator_hz(Waveform::Sine, &modulate * 2.0).build();
    voice(freq, gate)
        .filter(
            compress()
                .threshold(2.0)
                .scale(1.0 + &modulate * 8.0)
                .ratio(0.1)
                .build(),
        )
        .filter(
            low_pass_moog_ladder(6000.0 + &lfo * 2000.0)
                .resonance(1.0)
                .build(),
        )
}

fn kick(trigger: Trigger) -> Sf64 {
    let clock = trigger.to_gate();
    let duration_s = 0.1;
    let freq_hz = adsr_linear_01(&clock)
        .release_s(duration_s)
        .build()
        .exp_01(1.0)
        * 120;
    let osc = oscillator_hz(Waveform::Triangle, freq_hz).build();
    let env_amp = adsr_linear_01(&clock)
        .release_s(duration_s)
        .build()
        .exp_01(1.0)
        .filter(low_pass_moog_ladder(1000.0).build());
    osc.mul_lazy(&env_amp)
        .filter(compress().ratio(0.02).scale(16.0).build())
}

fn snare(trigger: Trigger) -> Sf64 {
    let clock = trigger.to_gate();
    let duration_s = 0.1;
    let noise = noise().filter(compress().ratio(0.1).scale(100.0).build());
    let env = adsr_linear_01(&clock)
        .release_s(duration_s * 1.0)
        .build()
        .exp_01(1.0)
        .filter(low_pass_moog_ladder(1000.0).build());
    let noise = noise
        .filter(low_pass_moog_ladder(10000.0).resonance(2.0).build())
        .filter(down_sample(10.0).build());
    let freq_hz = adsr_linear_01(&clock)
        .release_s(duration_s)
        .build()
        .exp_01(1.0)
        * 240;
    let osc = oscillator_hz(Waveform::Pulse, freq_hz)
        .reset_trigger(trigger)
        .build();
    (noise + osc)
        .filter(down_sample(10.0).build())
        .mul_lazy(&env)
}

fn cymbal(trigger: Trigger) -> Sf64 {
    let gate = trigger.to_gate();
    let osc = noise();
    let env = adsr_linear_01(gate)
        .release_s(0.1)
        .build()
        .filter(low_pass_butterworth(100.0).build());
    osc.filter(low_pass_moog_ladder(10000 * &env).build())
        .filter(high_pass_butterworth(6000.0).build())
}

fn drum_signal(trigger: Trigger) -> Sf64 {
    const CYMBAL: usize = 0;
    const SNARE: usize = 1;
    const KICK: usize = 2;
    let drum_pattern = {
        let cymbal = 1 << CYMBAL;
        let snare = 1 << SNARE;
        let kick = 1 << KICK;
        vec![
            cymbal | kick,
            cymbal,
            cymbal | snare,
            cymbal,
            cymbal | kick,
            cymbal,
            cymbal | snare,
            cymbal | snare,
            cymbal | kick,
            cymbal,
            cymbal | snare,
            cymbal,
            cymbal | kick,
            cymbal | kick,
            cymbal | snare,
            cymbal | snare,
        ]
    };
    let drum_sequence = bitwise_pattern_triggers_8(trigger, drum_pattern).triggers;
    match &drum_sequence.as_slice() {
        &[cymbal_trigger, snare_trigger, kick_trigger, ..] => {
            cymbal(cymbal_trigger.clone())
                + snare(snare_trigger.clone())
                + kick(kick_trigger.clone())
        }
        _ => panic!(),
    }
}

fn signal(trigger: Trigger) -> Sf64 {
    (synth_signal(trigger.divide(2)) / 2.0 + drum_signal(trigger.divide(3))) * 0.2
}

fn main() -> anyhow::Result<()> {
    let signal = signal(periodic_trigger_hz(10.0).build());
    let mut signal_player = SignalPlayer::new()?;
    signal_player.play_sample_forever(signal)
}
