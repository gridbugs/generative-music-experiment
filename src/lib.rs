use currawong::prelude::*;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;

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
        .filter(echo().scale(0.5).time_s(0.2).build())
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
    .filter(echo().scale(0.5).time_s(0.1).build())
}

fn signal() -> Sf64 {
    let trigger = periodic_trigger_hz(10.0).build();
    (synth_signal(trigger.divide(2)) / 2.0 + drum_signal(trigger.divide(3))) * 0.2
}

fn window() -> web_sys::Window {
    web_sys::window().expect("no global `window` exists")
}

fn document() -> web_sys::Document {
    window()
        .document()
        .expect("should have a document on window")
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {
    window()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .expect("should register `requestAnimationFrame` OK");
}

struct Synth {
    signal: Sf64,
    player: SignalPlayer,
    playing: bool,
}

impl Synth {
    fn new() -> Self {
        Self {
            signal: signal(),
            player: SignalPlayer::new().unwrap(),
            playing: true,
        }
    }
    fn send_signal(&mut self) {
        if self.playing {
            self.player.send_signal(&mut self.signal);
        } else {
            self.player.send_signal(&mut const_(0.0));
        }
    }
}

fn start_synth(synth: Rc<RefCell<Option<Synth>>>) {
    {
        let mut synth_opt = synth.borrow_mut();
        if let Some(synth) = synth_opt.as_mut() {
            synth.playing = true;
            return;
        } else {
            *synth_opt = Some(Synth::new());
        }
    }
    let loop_callback = Rc::new(RefCell::new(None));
    let loop_callback_ = Rc::clone(&loop_callback);
    *loop_callback_.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let mut synth_opt = synth.borrow_mut();
        let synth = synth_opt.as_mut().unwrap();
        synth.send_signal();
        request_animation_frame(loop_callback.borrow().as_ref().unwrap())
    }) as Box<dyn FnMut()>));
    loop_callback_
        .borrow()
        .as_ref()
        .unwrap()
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .call0(&JsValue::NULL)
        .unwrap();
}

fn stop_synth(synth: Rc<RefCell<Option<Synth>>>) {
    let mut synth_opt = synth.borrow_mut();
    if let Some(synth) = synth_opt.as_mut() {
        synth.playing = false;
    }
}

fn mk_button(text: &str) -> Result<web_sys::HtmlElement, JsValue> {
    let button = document().create_element("button")?;
    button.set_inner_html(text);
    let button = button.unchecked_into::<web_sys::HtmlElement>();
    let style = button.style();
    style.set_property("font-size", "24pt")?;
    Ok(button)
}

fn on_click<F: FnMut() + 'static>(element: &web_sys::HtmlElement, mut f: F) -> Result<(), JsValue> {
    let closure = Closure::wrap(Box::new(move |_event: JsValue| f()) as Box<dyn FnMut(JsValue)>);
    element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

#[wasm_bindgen(start)]
pub fn run() -> Result<(), JsValue> {
    wasm_logger::init(wasm_logger::Config::new(log::Level::Info));
    console_error_panic_hook::set_once();
    let synth = Rc::new(RefCell::new(None));
    let body = document().body().unwrap();
    let button_start = mk_button("Start")?;
    body.append_child(&button_start)?;
    on_click(&button_start, {
        let synth = Rc::clone(&synth);
        move || {
            start_synth(Rc::clone(&synth));
        }
    })?;
    let button_stop = mk_button("Stop")?;
    body.append_child(&button_stop)?;
    on_click(&button_stop, {
        let synth = Rc::clone(&synth);
        move || {
            stop_synth(Rc::clone(&synth));
        }
    })?;
    Ok(())
}
