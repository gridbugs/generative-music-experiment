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

fn voice(freq: Sfreq, gate: Gate, effect1: Sf64, effect2: Sf64) -> Sf64 {
    let freq_hz = freq.hz();
    let osc = oscillator_hz(Waveform::Saw, freq_hz.clone()).build();
    let env_amp = adsr_linear_01(&gate).attack_s(0.01).release_s(0.5).build();
    let env_lpf = adsr_linear_01(&gate)
        .attack_s(0.01)
        .release_s(0.5)
        .build()
        .exp_01(1.0);
    osc.filter(
        low_pass_moog_ladder(1000.0 + 2000.0 * env_lpf * effect1)
            .resonance(1.0 * &effect2)
            .build(),
    )
    .filter(compress().scale(effect2 * 4.0).build())
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
    let modulate = 1.0
        - oscillator_s(Waveform::Triangle, 60.0)
            .build()
            .signed_to_01();
    let effect1 = (1.0 - oscillator_s(Waveform::Sine, 47.0).build()).signed_to_01();
    let effect2 = oscillator_s(Waveform::Sine, 67.0)
        .reset_offset_01(-0.25)
        .build()
        .signed_to_01();
    let effect3 = oscillator_s(Waveform::Sine, 51.0).build().signed_to_01();
    let mk_voice = {
        |freq, trigger: Trigger| {
            let trigger = trigger.clone();
            let effect1 = effect1.clone();
            let effect2 = effect2.clone();
            let gate = trigger.to_gate_with_duration_s(0.02);
            voice(freq, gate, effect1.clone(), effect2.clone()).filter(
                compress()
                    .threshold(2.0)
                    .scale(1.0 + &modulate * 8.0)
                    .ratio(0.1)
                    .build(),
            )
        }
    };
    let poly_triggers = trigger_split_cycle(trigger, 2);
    let dry: Sf64 = poly_triggers
        .into_iter()
        .map(move |trigger| {
            let freq = random_replace_loop(
                trigger.clone(),
                const_(
                    Note {
                        name: NoteName::C,
                        octave: 1,
                    }
                    .freq(),
                ),
                random_note_c_major(const_(100.0), const_(300.0)),
                32,
                const_(0.1),
                const_(0.5),
            );
            mk_voice(freq, trigger.random_skip(0.5))
        })
        .sum();
    (dry.filter(reverb().room_size(1.0).build()) * 1.0 + (3.0 * effect3)) + dry
}

fn signal() -> Sf64 {
    let trigger = periodic_trigger_hz(1.0).build();
    synth_signal(trigger.divide(1)) * 0.2
}

fn window() -> web_sys::Window {
    web_sys::window().expect("no global `window` exists")
}

fn document() -> web_sys::Document {
    window()
        .document()
        .expect("should have a document on window")
}

fn set_timeout(f: &Closure<dyn FnMut()>, period_ms: i32) {
    let array = js_sys::Array::new();
    window()
        .set_timeout_with_callback_and_timeout_and_arguments(
            f.as_ref().unchecked_ref(),
            period_ms,
            &array,
        )
        .expect("failed to set timeout");
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
            player: SignalPlayer::new_with_downsample(1).unwrap(),
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
        set_timeout(loop_callback.borrow().as_ref().unwrap(), 16);
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
