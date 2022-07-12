use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::spawn;
use std::time::Duration;

use crate::{ButtonConfig, ButtonConfigs, MousesConfig};

use enigo::{Enigo, KeyboardControllable, MouseButton, MouseControllable};
use thread_priority::{set_current_thread_priority, ThreadPriority};
use util::config::ConfigManager;
use util::thread::CondMutex;
use util::time::Timer;
use util::tokenizer::{tokenize, Button, Key, StateToken, Token};

type ButtonConfigToken = [[StateToken; 3]; 2];

#[derive(Debug)]
pub struct ButtonConfigsToken {
    scroll_button: ButtonConfigToken,
    left_actionlock: ButtonConfigToken,
    right_actionlock: ButtonConfigToken,
    forwards_button: ButtonConfigToken,
    back_button: ButtonConfigToken,
    thumb_anticlockwise: ButtonConfigToken,
    thumb_clockwise: ButtonConfigToken,
    hat_top: ButtonConfigToken,
    hat_left: ButtonConfigToken,
    hat_right: ButtonConfigToken,
    hat_bottom: ButtonConfigToken,
    button_1: ButtonConfigToken,
    precision_aim: ButtonConfigToken,
    button_2: ButtonConfigToken,
    button_3: ButtonConfigToken,
}

impl ButtonConfigsToken {
    fn from_config(button_configs: ButtonConfigs) -> Self {
        Self {
            scroll_button: button_configs.scroll_button.tokenize(),
            left_actionlock: button_configs.left_actionlock.tokenize(),
            right_actionlock: button_configs.right_actionlock.tokenize(),
            forwards_button: button_configs.forwards_button.tokenize(),
            back_button: button_configs.back_button.tokenize(),
            thumb_anticlockwise: button_configs.thumb_anticlockwise.tokenize(),
            thumb_clockwise: button_configs.thumb_clockwise.tokenize(),
            hat_top: button_configs.hat_top.tokenize(),
            hat_left: button_configs.hat_left.tokenize(),
            hat_right: button_configs.hat_right.tokenize(),
            hat_bottom: button_configs.hat_bottom.tokenize(),
            button_1: button_configs.button_1.tokenize(),
            precision_aim: button_configs.precision_aim.tokenize(),
            button_2: button_configs.button_2.tokenize(),
            button_3: button_configs.button_3.tokenize(),
        }
    }
}

struct ClickState {
    left: bool,
    right: bool,
    middle: bool,
}

struct ButtonState {
    scroll_button: bool,
    left_actionlock: bool,
    right_actionlock: bool,
    forwards_button: bool,
    back_button: bool,
    thumb_anticlockwise: bool,
    thumb_clockwise: bool,
    hat_top: bool,
    hat_left: bool,
    hat_right: bool,
    hat_bottom: bool,
    button_1: bool,
    precision_aim: bool,
    button_2: bool,
    button_3: bool,
}

struct ButtonTimer {
    scroll_button: Rc<RefCell<Timer>>,
    left_actionlock: Rc<RefCell<Timer>>,
    right_actionlock: Rc<RefCell<Timer>>,
    forwards_button: Rc<RefCell<Timer>>,
    back_button: Rc<RefCell<Timer>>,
    thumb_anticlockwise: Rc<RefCell<Timer>>,
    thumb_clockwise: Rc<RefCell<Timer>>,
    hat_top: Rc<RefCell<Timer>>,
    hat_left: Rc<RefCell<Timer>>,
    hat_right: Rc<RefCell<Timer>>,
    hat_bottom: Rc<RefCell<Timer>>,
    button_1: Rc<RefCell<Timer>>,
    precision_aim: Rc<RefCell<Timer>>,
    button_2: Rc<RefCell<Timer>>,
    button_3: Rc<RefCell<Timer>>,
}

enum Mode {
    Normal(u8),
    Shift(u8),
}

pub struct Mapper {
    enigo: Enigo,
    mode: Mode,
    click_state: ClickState,
    button_state: ButtonState,
    button_timer: ButtonTimer,
    button_configs_token: ButtonConfigsToken,
    mouses_config_mutex: Arc<tokio::sync::Mutex<ConfigManager<MousesConfig>>>,
    mouses_config_state_id: Arc<AtomicU32>,
    last_mouses_config_state_id: u32,
    serial_number: String,
    emulation_worker_rx: Sender<Vec<Token>>,
    mouse_relative_movement_condmutex: Arc<CondMutex<(i32, i32)>>,
}

impl Mapper {
    pub fn new(
        mouses_config_mutex: Arc<tokio::sync::Mutex<ConfigManager<MousesConfig>>>,
        mouses_config_state_id: Arc<AtomicU32>,
        serial_number: String,
    ) -> Self {
        let last_mouses_config_state_id = mouses_config_state_id.load(Ordering::SeqCst);
        let button_configs = mouses_config_mutex.blocking_lock().config[&serial_number].clone();
        let (emulation_worker_rx, emulation_worker_tx) = channel();
        let mouse_relative_movement_condmutex = Arc::new(CondMutex::new((0, 0)));
        let mouse_relative_movement_condmutex_clone = mouse_relative_movement_condmutex.clone();

        // mouse movement worker
        spawn(move || {
            set_current_thread_priority(ThreadPriority::Max).ok();

            let mut enigo = Enigo::new();

            loop {
                let mouse_relative_movement = {
                    let mut mouse_relative_movement =
                        mouse_relative_movement_condmutex_clone.wait_poisoned();
                    let mouse_relative_movement_clone = mouse_relative_movement.clone();

                    *mouse_relative_movement = (0, 0);
                    mouse_relative_movement_clone
                };

                enigo.mouse_move_relative(mouse_relative_movement.0, mouse_relative_movement.1);
            }
        });

        // emulation worker
        spawn(move || {
            set_current_thread_priority(ThreadPriority::Max).ok();

            let mut enigo = Enigo::new();

            while let Ok(token_vec) = emulation_worker_tx.recv() {
                emulate_token_vec(&mut enigo, token_vec);
            }
        });

        Self {
            enigo: Enigo::new(),
            mode: Mode::Normal(0),
            click_state: ClickState {
                left: false,
                right: false,
                middle: false,
            },
            button_state: ButtonState {
                back_button: false,
                forwards_button: false,
                button_1: false,
                button_2: false,
                button_3: false,
                hat_top: false,
                hat_bottom: false,
                hat_left: false,
                hat_right: false,
                precision_aim: false,
                thumb_clockwise: false,
                thumb_anticlockwise: false,
                scroll_button: false,
                left_actionlock: false,
                right_actionlock: false,
            },
            button_timer: ButtonTimer {
                back_button: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                forwards_button: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                button_1: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                button_2: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                button_3: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                hat_top: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                hat_bottom: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                hat_left: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                hat_right: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                precision_aim: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                thumb_clockwise: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                thumb_anticlockwise: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                scroll_button: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                left_actionlock: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
                right_actionlock: Rc::new(RefCell::new(Timer::new(Duration::from_millis(50)))),
            },
            button_configs_token: ButtonConfigsToken::from_config(button_configs),
            mouses_config_mutex,
            mouses_config_state_id,
            last_mouses_config_state_id,
            serial_number,
            emulation_worker_rx,
            mouse_relative_movement_condmutex,
        }
    }

    pub fn emulate(&mut self, buffer: &[u8]) {
        if self.config_has_change() {
            self.button_configs_token = ButtonConfigsToken::from_config(
                self.mouses_config_mutex.blocking_lock().config[&self.serial_number].clone(),
            );
        }

        self.update_mode(buffer);
        self.basic_emulation(buffer);
        self.mapped_emulation(buffer);
    }

    pub fn emulate_only_mapped(&mut self, buffer: &[u8]) {
        if self.config_has_change() {
            self.button_configs_token = ButtonConfigsToken::from_config(
                self.mouses_config_mutex.blocking_lock().config[&self.serial_number].clone(),
            );
        }

        self.mapped_emulation(buffer);
    }

    fn update_mode(&mut self, buffer: &[u8]) {
        let modes = buffer[2] & 0b111;

        self.mode = match modes {
            0 | 1 | 2 => Mode::Normal(modes),
            4 | 5 | 6 => Mode::Shift(modes - 0b100),
            _ => Mode::Normal(0),
        };
    }

    fn basic_emulation(&mut self, buffer: &[u8]) {
        // button emulation
        let click_state = ClickState {
            left: (buffer[0] & 1) > 0,
            right: (buffer[0] & 2) > 0,
            middle: (buffer[0] & 4) > 0,
        };
        let middle_button_state_token =
            self.get_state_token(&self.button_configs_token.scroll_button);

        if click_state.left != self.click_state.left {
            self.click_state.left = click_state.left;

            if click_state.left {
                self.enigo.mouse_down(MouseButton::Left);
            } else {
                self.enigo.mouse_up(MouseButton::Left);
            }
        }
        if middle_button_state_token.down.is_empty()
            && middle_button_state_token.repeat.is_empty()
            && middle_button_state_token.up.is_empty()
        {
            if click_state.middle != self.click_state.middle {
                self.click_state.middle = click_state.middle;

                if click_state.middle {
                    self.enigo.mouse_down(MouseButton::Middle);
                } else {
                    self.enigo.mouse_up(MouseButton::Middle);
                }
            }
        }
        if click_state.right != self.click_state.right {
            self.click_state.right = click_state.right;

            if click_state.right {
                self.enigo.mouse_down(MouseButton::Right);
            } else {
                self.enigo.mouse_up(MouseButton::Right);
            }
        }

        // movement emulation
        {
            let mut mouse_relative_movement =
                self.mouse_relative_movement_condmutex.lock_poisoned();

            mouse_relative_movement.0 += if buffer[3] < 128 {
                buffer[3] as i32
            } else {
                buffer[3] as i32 - 256
            };
            mouse_relative_movement.1 += if buffer[5] < 128 {
                buffer[5] as i32
            } else {
                buffer[5] as i32 - 256
            };

            self.mouse_relative_movement_condmutex.notify_one();
        }

        // wheel emulation
        if buffer[7] == 1 {
            self.enigo.mouse_scroll_y(-1);
        }
        if buffer[7] == 255 {
            self.enigo.mouse_scroll_y(1);
        }
    }

    fn mapped_emulation(&mut self, buffer: &[u8]) {
        let button_state = ButtonState {
            back_button: (buffer[0] & 8) > 0,
            forwards_button: (buffer[0] & 16) > 0,
            button_1: (buffer[0] & 32) > 0,
            button_2: (buffer[0] & 64) > 0,
            button_3: (buffer[0] & 128) > 0,
            hat_top: (buffer[1] & 1) > 0,
            hat_bottom: (buffer[1] & 2) > 0,
            hat_left: (buffer[1] & 4) > 0,
            hat_right: (buffer[1] & 8) > 0,
            precision_aim: (buffer[1] & 16) > 0,
            thumb_clockwise: (buffer[1] & 32) > 0,
            thumb_anticlockwise: (buffer[1] & 64) > 0,
            scroll_button: (buffer[2] & 8) > 0,
            left_actionlock: (buffer[2] & 16) > 0,
            right_actionlock: (buffer[2] & 32) > 0,
        };

        self.emulate_button_config_token(
            self.button_configs_token.back_button.clone(),
            self.button_timer.back_button.clone(),
            self.button_state.back_button,
            button_state.back_button,
        );
        self.emulate_button_config_token(
            self.button_configs_token.forwards_button.clone(),
            self.button_timer.forwards_button.clone(),
            self.button_state.forwards_button,
            button_state.forwards_button,
        );
        self.emulate_button_config_token(
            self.button_configs_token.button_1.clone(),
            self.button_timer.button_1.clone(),
            self.button_state.button_1,
            button_state.button_1,
        );
        self.emulate_button_config_token(
            self.button_configs_token.button_2.clone(),
            self.button_timer.button_2.clone(),
            self.button_state.button_2,
            button_state.button_2,
        );
        self.emulate_button_config_token(
            self.button_configs_token.button_3.clone(),
            self.button_timer.button_3.clone(),
            self.button_state.button_3,
            button_state.button_3,
        );
        self.emulate_button_config_token(
            self.button_configs_token.hat_top.clone(),
            self.button_timer.hat_top.clone(),
            self.button_state.hat_top,
            button_state.hat_top,
        );
        self.emulate_button_config_token(
            self.button_configs_token.hat_bottom.clone(),
            self.button_timer.hat_bottom.clone(),
            self.button_state.hat_bottom,
            button_state.hat_bottom,
        );
        self.emulate_button_config_token(
            self.button_configs_token.hat_left.clone(),
            self.button_timer.hat_left.clone(),
            self.button_state.hat_left,
            button_state.hat_left,
        );
        self.emulate_button_config_token(
            self.button_configs_token.hat_right.clone(),
            self.button_timer.hat_right.clone(),
            self.button_state.hat_right,
            button_state.hat_right,
        );
        self.emulate_button_config_token(
            self.button_configs_token.precision_aim.clone(),
            self.button_timer.precision_aim.clone(),
            self.button_state.precision_aim,
            button_state.precision_aim,
        );
        self.emulate_button_config_token(
            self.button_configs_token.thumb_clockwise.clone(),
            self.button_timer.thumb_clockwise.clone(),
            self.button_state.thumb_clockwise,
            button_state.thumb_clockwise,
        );
        self.emulate_button_config_token(
            self.button_configs_token.thumb_anticlockwise.clone(),
            self.button_timer.thumb_anticlockwise.clone(),
            self.button_state.thumb_anticlockwise,
            button_state.thumb_anticlockwise,
        );
        self.emulate_button_config_token(
            self.button_configs_token.scroll_button.clone(),
            self.button_timer.scroll_button.clone(),
            self.button_state.scroll_button,
            button_state.scroll_button,
        );
        self.emulate_button_config_token(
            self.button_configs_token.left_actionlock.clone(),
            self.button_timer.left_actionlock.clone(),
            self.button_state.left_actionlock,
            button_state.left_actionlock,
        );
        self.emulate_button_config_token(
            self.button_configs_token.right_actionlock.clone(),
            self.button_timer.right_actionlock.clone(),
            self.button_state.right_actionlock,
            button_state.right_actionlock,
        );

        self.button_state = button_state;
    }

    fn is_shift_mode(&self) -> bool {
        match self.mode {
            Mode::Normal(_) => false,
            Mode::Shift(_) => true,
        }
    }

    fn absolute_mode(&self) -> u8 {
        match self.mode {
            Mode::Normal(mode) => mode,
            Mode::Shift(mode) => mode,
        }
    }

    fn config_has_change(&mut self) -> bool {
        let mouses_config_state_id = self.mouses_config_state_id.load(Ordering::SeqCst);

        if self.last_mouses_config_state_id != mouses_config_state_id {
            self.last_mouses_config_state_id = mouses_config_state_id;

            true
        } else {
            false
        }
    }

    fn get_state_token(&self, button_config_token: &ButtonConfigToken) -> StateToken {
        button_config_token[self.is_shift_mode() as usize][self.absolute_mode() as usize].clone()
    }

    fn emulate_button_config_token(
        &mut self,
        button_config_token: ButtonConfigToken,
        button_timer: Rc<RefCell<Timer>>,
        previous_button_state: bool,
        current_button_state: bool,
    ) {
        let state_token = self.get_state_token(&button_config_token);

        if current_button_state != previous_button_state {
            if current_button_state {
                self.emulation_worker_rx.send(state_token.down).ok();
            } else {
                self.emulation_worker_rx.send(state_token.up).ok();
            }
        }

        if button_timer.borrow_mut().check() && current_button_state {
            self.emulation_worker_rx.send(state_token.repeat).ok();
        }
    }
}

trait ButtonConfigExt {
    fn tokenize(&self) -> ButtonConfigToken;
}

impl ButtonConfigExt for ButtonConfig {
    fn tokenize(&self) -> ButtonConfigToken {
        let mut button_config_token = [
            [
                StateToken::default(),
                StateToken::default(),
                StateToken::default(),
            ],
            [
                StateToken::default(),
                StateToken::default(),
                StateToken::default(),
            ],
        ];

        for mode_type_index in 0..2 {
            for mode_index in 0..3 {
                if let Some(config) = self[mode_type_index].get(mode_index) {
                    button_config_token[mode_type_index][mode_index] = tokenize(config.clone());
                }
            }
        }

        button_config_token
    }
}

fn emulate_token_vec(enigo: &mut Enigo, token_vec: Vec<Token>) {
    fn key_to_enigo(key: Key) -> enigo::Key {
        match key {
            Key::Shift => enigo::Key::Shift,
            Key::Control => enigo::Key::Control,
            Key::Alt => enigo::Key::Alt,
            Key::Command => enigo::Key::Meta,
        }
    }

    for token in token_vec {
        match token {
            Token::Sequence(sequence) => {
                for key in sequence.chars() {
                    enigo.key_click(enigo::Key::Layout(key));
                }
            }
            Token::Unicode(unicode_sequence) => enigo.key_sequence(unicode_sequence.as_str()),
            Token::KeyUp(key) => enigo.key_up(key_to_enigo(key)),
            Token::KeyDown(key) => enigo.key_down(key_to_enigo(key)),
            Token::MouseUp(button) => match button {
                Button::Left => enigo.mouse_up(enigo::MouseButton::Left),
                Button::Middle => enigo.mouse_up(enigo::MouseButton::Middle),
                Button::Right => enigo.mouse_up(enigo::MouseButton::Right),
                _ => {}
            },
            Token::MouseDown(button) => match button {
                Button::Left => enigo.mouse_down(enigo::MouseButton::Left),
                Button::Middle => enigo.mouse_down(enigo::MouseButton::Middle),
                Button::Right => enigo.mouse_down(enigo::MouseButton::Right),
                _ => {}
            },
            Token::Click(button) => match button {
                Button::Left => enigo.mouse_click(enigo::MouseButton::Left),
                Button::Middle => enigo.mouse_click(enigo::MouseButton::Middle),
                Button::Right => enigo.mouse_click(enigo::MouseButton::Right),
                Button::ScrollUp => enigo.mouse_scroll_y(1),
                Button::ScrollDown => enigo.mouse_scroll_y(-1),
                Button::ScrollLeft => enigo.mouse_scroll_x(1),
                Button::ScrollRight => enigo.mouse_scroll_x(-1),
            },
        }
    }
}
