// hide the console on release builds for windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mapper;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::Duration;

use hashbrown::HashSet;
use mapper::Mapper;
use rusb::{Context, DeviceHandle, UsbContext};
use serde::{Deserialize, Serialize};
use thread_priority::{set_current_thread_priority, ThreadPriority};
use tokio::time::{interval, MissedTickBehavior};
use util::config::ConfigManager;
use util::connection::{command::*, Client, ConnectionState};
use util::thread::{kill_double, DualChannel, MutexTrait};
use util::time::TIMEOUT_1S;

const VID: u16 = 0x0738;
const PID: u16 = 0x1713;

type ButtonConfig = [Vec<String>; 2];

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
pub struct ButtonConfigs {
    scroll_button: ButtonConfig,
    left_actionlock: ButtonConfig,
    right_actionlock: ButtonConfig,
    forwards_button: ButtonConfig,
    back_button: ButtonConfig,
    thumb_anticlockwise: ButtonConfig,
    thumb_clockwise: ButtonConfig,
    hat_top: ButtonConfig,
    hat_left: ButtonConfig,
    hat_right: ButtonConfig,
    hat_bottom: ButtonConfig,
    button_1: ButtonConfig,
    precision_aim: ButtonConfig,
    button_2: ButtonConfig,
    button_3: ButtonConfig,
}

impl ButtonConfigs {
    fn to_config(&self) -> Vec<ButtonConfig> {
        vec![
            self.scroll_button.clone(),
            self.left_actionlock.clone(),
            self.right_actionlock.clone(),
            self.forwards_button.clone(),
            self.back_button.clone(),
            self.thumb_anticlockwise.clone(),
            self.thumb_clockwise.clone(),
            self.hat_top.clone(),
            self.hat_left.clone(),
            self.hat_right.clone(),
            self.hat_bottom.clone(),
            self.button_1.clone(),
            self.precision_aim.clone(),
            self.button_2.clone(),
            self.button_3.clone(),
        ]
    }

    fn from_config(data: &Vec<ButtonConfig>) -> Self {
        Self {
            scroll_button: data[0].clone(),
            left_actionlock: data[1].clone(),
            right_actionlock: data[2].clone(),
            forwards_button: data[3].clone(),
            back_button: data[4].clone(),
            thumb_anticlockwise: data[5].clone(),
            thumb_clockwise: data[6].clone(),
            hat_top: data[7].clone(),
            hat_left: data[8].clone(),
            hat_right: data[9].clone(),
            hat_bottom: data[10].clone(),
            button_1: data[11].clone(),
            precision_aim: data[12].clone(),
            button_2: data[13].clone(),
            button_3: data[14].clone(),
        }
    }
}

type MousesConfig = BTreeMap<String, ButtonConfigs>;

#[derive(Debug)]
struct Endpoint {
    config: u8,
    iface: u8,
    setting: u8,
    address: u8,
}

#[derive(Debug, Clone)]
enum Message {
    DeviceListUpdate,
}

#[tokio::main]
async fn main() {
    if kill_double() {
        return;
    }

    set_current_thread_priority(ThreadPriority::Min).ok();

    let client = Client::new().await;
    let client_dualchannel = client.dual_channel;
    let device_list_mutex = Arc::new(Mutex::new(HashSet::<String>::new()));
    let (host, child) = DualChannel::<Message>::new();
    let icon_data = include_bytes!("../icon.png").to_vec();
    let mouses_config_mutex = Arc::new(tokio::sync::Mutex::new(
        ConfigManager::<MousesConfig>::new("mmo7_profiles"),
    ));
    let mouses_config_state_id = Arc::new(AtomicU32::new(0));

    watch_config_update(mouses_config_mutex.clone(), mouses_config_state_id.clone()).await;
    run_connection(
        client_dualchannel,
        child,
        device_list_mutex.clone(),
        icon_data,
        mouses_config_mutex.clone(),
        mouses_config_state_id.clone(),
    )
    .await;
    listening_new_device(
        host,
        device_list_mutex,
        mouses_config_mutex,
        mouses_config_state_id,
    )
    .await;
}

async fn watch_config_update(
    mouses_config_mutex: Arc<tokio::sync::Mutex<ConfigManager<MousesConfig>>>,
    mouses_config_state_id: Arc<AtomicU32>,
) {
    let mouses_config_mutex = mouses_config_mutex.clone();

    tokio::spawn(async move {
        let mut interval_ = interval(TIMEOUT_1S * 10);

        loop {
            if mouses_config_mutex.lock().await.update() {
                mouses_config_state_id.fetch_add(1, Ordering::SeqCst);
            }

            interval_.tick().await;
        }
    });
}

// device handling
async fn listening_new_device(
    host: DualChannel<Message>,
    device_list_mutex: Arc<Mutex<HashSet<String>>>,
    mouses_config_mutex: Arc<tokio::sync::Mutex<ConfigManager<MousesConfig>>>,
    mouses_config_state_id: Arc<AtomicU32>,
) {
    let mut interval_ = interval(TIMEOUT_1S);

    interval_.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        if let Ok(context) = Context::new() {
            if let Ok(devices) = context.devices() {
                for device in devices.iter() {
                    if let Ok(device_descriptor) = device.device_descriptor() {
                        if device_descriptor.vendor_id() == VID
                            && device_descriptor.product_id() == PID
                        {
                            if let Ok(device_handle) = device.open() {
                                if let Ok(languages) =
                                    device_handle.read_languages(Duration::from_millis(100))
                                {
                                    if let Ok(serial_number) = device_handle
                                        .read_serial_number_string(
                                            languages[0],
                                            &device_descriptor,
                                            Duration::from_millis(100),
                                        )
                                    {
                                        let mut device_list = device_list_mutex.lock_poisoned();

                                        if let None = device_list.get(&serial_number) {
                                            {
                                                // create a default config if needed
                                                let mut mouses_config =
                                                    mouses_config_mutex.lock().await;

                                                if !mouses_config
                                                    .config
                                                    .contains_key(&serial_number)
                                                {
                                                    mouses_config.config.insert(
                                                        serial_number.clone(),
                                                        ButtonConfigs::default(),
                                                    );
                                                    mouses_config.save();
                                                }
                                            }

                                            device_list.insert(serial_number.clone());

                                            let host = host.clone();
                                            let device_list_mutex = device_list_mutex.clone();
                                            let mouses_config_mutex = mouses_config_mutex.clone();
                                            let mouses_config_state_id =
                                                mouses_config_state_id.clone();

                                            spawn(move || {
                                                set_current_thread_priority(ThreadPriority::Max)
                                                    .ok();

                                                run_device(
                                                    serial_number.clone(),
                                                    host.clone(),
                                                    mouses_config_mutex,
                                                    mouses_config_state_id,
                                                );

                                                device_list_mutex
                                                    .lock_poisoned()
                                                    .remove(&serial_number);
                                                host.send(Message::DeviceListUpdate).ok();
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        interval_.tick().await;
    }
}

fn find_device(serial_number: String) -> Option<DeviceHandle<Context>> {
    if let Ok(context) = Context::new() {
        if let Ok(devices) = context.devices() {
            for device in devices.iter() {
                if let Ok(device_descriptor) = device.device_descriptor() {
                    if device_descriptor.vendor_id() == VID && device_descriptor.product_id() == PID
                    {
                        if let Ok(device_handle) = device.open() {
                            if let Ok(languages) = device_handle.read_languages(TIMEOUT_1S) {
                                if let Ok(serial_number_found) = device_handle
                                    .read_serial_number_string(
                                        languages[0],
                                        &device_descriptor,
                                        TIMEOUT_1S,
                                    )
                                {
                                    if serial_number == serial_number_found {
                                        return Some(device_handle);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn run_device(
    serial_number: String,
    dual_channel: DualChannel<Message>,
    mouses_config_mutex: Arc<tokio::sync::Mutex<ConfigManager<MousesConfig>>>,
    mouses_config_state_id: Arc<AtomicU32>,
) {
    if let Some(mut device_handle) = find_device(serial_number.clone()) {
        let device = device_handle.device();
        if let Ok(config_descriptor) = device.config_descriptor(0) {
            if let Some(interface) = config_descriptor.interfaces().next() {
                if let Some(interface_descriptor) = interface.descriptors().next() {
                    if let Some(endpoint_descriptor) =
                        interface_descriptor.endpoint_descriptors().next()
                    {
                        let endpoint = Endpoint {
                            config: config_descriptor.number(),
                            iface: interface_descriptor.interface_number(),
                            setting: interface_descriptor.setting_number(),
                            address: endpoint_descriptor.address(),
                        };

                        let has_kernel_driver =
                            match device_handle.kernel_driver_active(endpoint.iface) {
                                Ok(true) => {
                                    device_handle.detach_kernel_driver(endpoint.iface).ok();
                                    true
                                }
                                _ => false,
                            };

                        if let (Ok(_), Ok(_), Ok(_)) = (
                            device_handle.set_active_configuration(endpoint.config),
                            device_handle.claim_interface(endpoint.iface),
                            device_handle.set_alternate_setting(endpoint.iface, endpoint.setting),
                        ) {
                            println!("{} connected", serial_number);

                            dual_channel.send(Message::DeviceListUpdate).ok();

                            let mut buffer = [0; 8];
                            let mut mapper = Mapper::new(
                                mouses_config_mutex,
                                mouses_config_state_id,
                                serial_number.clone(),
                            );

                            loop {
                                match device_handle.read_interrupt(
                                    endpoint.address,
                                    &mut buffer,
                                    Duration::from_millis(25),
                                ) {
                                    Ok(_) => mapper.emulate(&buffer),
                                    Err(rusb::Error::Timeout) => {
                                        mapper.emulate_only_mapped(&buffer)
                                    }
                                    Err(err) => {
                                        println!("{} disconnected : {}", serial_number, err);
                                        break;
                                    }
                                }
                            }

                            if has_kernel_driver {
                                device_handle.attach_kernel_driver(endpoint.iface).ok();
                            }
                        }
                    }
                }
            }
        }
    }
}

// connection processing
async fn run_connection(
    client_dualchannel: DualChannel<ConnectionState>,
    child: DualChannel<Message>,
    device_list_mutex: Arc<Mutex<HashSet<String>>>,
    icon_data: Vec<u8>,
    mouses_config_mutex: Arc<tokio::sync::Mutex<ConfigManager<MousesConfig>>>,
    mouses_config_state_id: Arc<AtomicU32>,
) {
    {
        let client_dualchannel = client_dualchannel.clone();
        let device_list_mutex = device_list_mutex.clone();

        tokio::spawn(async move {
            let mut driver_configuration_descriptor = DriverConfigurationDescriptor::new(
                VID,
                PID,
                "MMO7".to_string(),
                icon_data,
                3,
                3,
                vec![
                    "Scroll Button".to_string(),
                    "Left ActionLock".to_string(),
                    "Right ActionLock".to_string(),
                    "Forwards Button".to_string(),
                    "Back Button".to_string(),
                    "Thumb Anticlockwise".to_string(),
                    "Thumb Clockwise".to_string(),
                    "Hat Top".to_string(),
                    "Hat Left".to_string(),
                    "Hat Right".to_string(),
                    "Hat Bottom".to_string(),
                    "Button 1".to_string(),
                    "Button 2".to_string(),
                    "Precision Aim".to_string(),
                    "Button 3".to_string(),
                ],
            );

            loop {
                if let Ok(connection_state) = client_dualchannel.recv_async().await {
                    match connection_state {
                        ConnectionState::Start => {
                            client_dualchannel
                                .send_async(ConnectionState::Data(
                                    driver_configuration_descriptor.to_bytes(),
                                ))
                                .await
                                .ok();

                            update_device_list(&client_dualchannel, device_list_mutex.clone())
                                .await;
                        }
                        ConnectionState::Data(data) => match Commands::from(data) {
                            Commands::RequestDeviceConfig(request_device_config) => {
                                let mouses_config = mouses_config_mutex.lock().await;

                                if let Some(mouse_config) = mouses_config
                                    .config
                                    .get(&request_device_config.serial_number)
                                {
                                    client_dualchannel
                                        .send_async(ConnectionState::Data(
                                            DeviceConfig::new(
                                                request_device_config.serial_number,
                                                mouse_config.to_config(),
                                            )
                                            .to_bytes(),
                                        ))
                                        .await
                                        .ok();
                                }
                            }
                            Commands::DeviceConfig(device_config) => {
                                let mut mouses_config = mouses_config_mutex.lock().await;

                                mouses_config.config.insert(
                                    device_config.serial_number,
                                    ButtonConfigs::from_config(&device_config.config),
                                );
                                mouses_config_state_id.fetch_add(1, Ordering::SeqCst);
                            }
                            _ => {}
                        },
                        ConnectionState::End => {}
                    }
                }
            }
        })
    };

    tokio::spawn(async move {
        loop {
            if let Ok(message) = child.recv_async().await {
                match message {
                    Message::DeviceListUpdate => {
                        update_device_list(&client_dualchannel, device_list_mutex.clone()).await;
                    }
                }
            }
        }
    });
}

async fn update_device_list(
    client_dualchannel: &DualChannel<ConnectionState>,
    device_list_mutex: Arc<Mutex<HashSet<String>>>,
) {
    let mut serial_number_vec = vec![];

    for serial_number in device_list_mutex.lock_poisoned().iter() {
        serial_number_vec.push(serial_number.clone());
    }

    client_dualchannel
        .send_async(ConnectionState::Data(
            DeviceList::new(serial_number_vec).to_bytes(),
        ))
        .await
        .ok();
}
