#![feature(try_blocks)]
use anyhow::{Error, Ok, Result};
use ash::{
    extensions,
    vk::{
        self, DebugUtilsMessageSeverityFlagsEXT, DebugUtilsMessageTypeFlagsEXT,
        DebugUtilsMessengerCreateInfoEXT,
    },
    Entry, Instance,
};
use std::ptr;
use std::{
    ffi::{c_void, CStr, CString},
    os::raw::c_char,
};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{self, Window, WindowBuilder},
};

use vulky::{
    constant::{validation, version},
    device::{create_logical_device, pick_phyiscal_device},
    platform,
};

/// The Vulkan SDK version that started requiring the portability subset extension for macOS.
pub const PORTABILITY_MACOS_VERSION: u32 = vk::make_api_version(0, 1, 3, 216);
fn main() -> Result<()> {
    // Create an event loop and window using winit
    unsafe {
        let event_loop = EventLoop::new();
        let window = WindowBuilder::new()
            .with_title("Vulkan Window")
            .build(&event_loop)
            .unwrap();

        let mut app = VulkanApp::new(&window)?;

        event_loop.run(move |event, _, control_flow| {
            // ControlFlow::Poll continuously runs the event loop, even if the OS hasn't
            // dispatched any events. This is ideal for games and similar applications.
            control_flow.set_poll();

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    println!("The close button was pressed; stopping");
                    app.destroy();
                    control_flow.set_exit();
                }
                Event::MainEventsCleared => {
                    // Application update code.
                    // Queue a RedrawRequested event.
                    //
                    // You only need to call this if you've determined that you need to redraw, in
                    // applications which do not always need to. Applications that redraw continuously
                    // can just render here instead.
                    window.request_redraw();
                }
                Event::RedrawRequested(_) => {
                    // Redraw the application.
                    //
                    // It's preferable for applications that do not render continuously to render in
                    // this event rather than in MainEventsCleared, since rendering in here allows
                    // the program to gracefully handle redraws requested by the OS.
                }
                _ => (),
            }
        });
    }

    // Get the HINSTANCE and HWND handles from the window on Windows
    Ok(())
}

struct VulkanApp {
    instance: ash::Instance,
    entry: ash::Entry,
    debug_util_loader: ash::extensions::ext::DebugUtils,
    debug_messenger: vk::DebugUtilsMessengerEXT,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    graphics_queue: vk::Queue,

    //Surface
    surface_loader: ash::extensions::khr::Surface,
    surface: vk::SurfaceKHR,
}
impl VulkanApp {
    unsafe fn new(window: &Window) -> Result<Self> {
        let entry = ash::Entry::load()?;
        let instance = create_instance(&entry)?;
        let (debug_util_loader, debug_messenger) = setup_debug_utils(&entry, &instance)?;
        let (physical_device, graphic_family) = pick_phyiscal_device(&entry, &instance)?;
        let device = create_logical_device(physical_device, &instance, graphic_family)?;
        let graphics_queue = device.get_device_queue(graphic_family, 0);
        let (surface, surface_loader) = create_surface(&entry, &instance, window)?;
        Ok(Self {
            instance,
            entry,
            physical_device,
            device,
            graphics_queue,
            surface,
            surface_loader,
            debug_util_loader,
            debug_messenger,
        })
    }

    unsafe fn render(&mut self) {}
    unsafe fn destroy(&mut self) {
        if validation::ENABLED {
            self.debug_util_loader
                .destroy_debug_utils_messenger(self.debug_messenger, None);
        }
        self.device.destroy_device(None);
        self.instance.destroy_instance(None);
    }
}

unsafe fn create_instance(entry: &ash::Entry) -> Result<ash::Instance> {
    let app_name = CString::new("window_title").unwrap();
    let engine_name = CString::new("Vulkan Engine").unwrap();

    if validation::ENABLED && !check_validation_support(&entry)? {
        panic!("Validation layer is requested, but no available");
    }

    let app_info = vk::ApplicationInfo::builder()
        .engine_name(&engine_name)
        .application_name(&app_name)
        .api_version(version::API_VERSION)
        .engine_version(version::ENGINE_VERSION)
        .application_version(version::APPLICATION_VERSION)
        .build();

    let mut extension = vulky::platform::required_extension_names();

    let layer_names = [CStr::from_bytes_with_nul_unchecked(
        validation::LAYER_NAME_BYTES,
    )];
    let layers_names_raw: Vec<*const c_char> = layer_names
        .iter()
        .map(|raw_name| raw_name.as_ptr())
        .collect();

    //macos portability
    let flags = if cfg!(target_os = "macos") && PORTABILITY_MACOS_VERSION >= version::API_VERSION {
        extension.push(ash::vk::KhrGetPhysicalDeviceProperties2Fn::name().as_ptr());
        extension.push(ash::vk::KhrPortabilityEnumerationFn::name().as_ptr());
        vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
    } else {
        vk::InstanceCreateFlags::empty()
    };

    let mut instance_info = vk::InstanceCreateInfo {
        s_type: vk::StructureType::INSTANCE_CREATE_INFO,
        p_next: ptr::null(),
        flags,
        p_application_info: &app_info,
        pp_enabled_layer_names: ptr::null(),
        enabled_layer_count: 0,
        enabled_extension_count: extension.len() as u32,
        pp_enabled_extension_names: extension.as_ptr(),
    };

    if validation::ENABLED {
        let debug_utils_create_info = debug_create_info()?;
        instance_info.p_next = &debug_utils_create_info
            as *const vk::DebugUtilsMessengerCreateInfoEXT
            as *const c_void;

        instance_info.pp_enabled_layer_names = layers_names_raw.as_ptr();
        instance_info.enabled_layer_count = layers_names_raw.len() as u32;
    }

    let instance = entry.create_instance(&instance_info, None)?;
    Ok(instance)
}

unsafe fn create_surface(
    entry: &Entry,
    instance: &Instance,
    window: &Window,
) -> Result<(ash::vk::SurfaceKHR, ash::extensions::khr::Surface)> {
    let surface = platform::create_surface(entry, instance, window)?;
    let surface_loader = ash::extensions::khr::Surface::new(entry, instance);

    Ok((surface, surface_loader))
}

unsafe fn check_validation_support(entry: &Entry) -> Result<bool> {
    let layer_properties = entry.enumerate_instance_layer_properties()?;
    let mut is_layer_found = false;

    for layer_property in layer_properties.iter() {
        let raw_string = layer_property.layer_name.as_ptr();
        let s = CStr::from_ptr(raw_string).to_str()?.to_owned();

        if s == validation::LAYER_NAME {
            is_layer_found = true;
            break;
        }
    }
    if !is_layer_found {
        eprintln!("Required Layer is not found");
        return Ok(false);
    }

    Ok(is_layer_found)
}

fn setup_debug_utils(
    entry: &ash::Entry,
    instance: &ash::Instance,
) -> Result<(ash::extensions::ext::DebugUtils, vk::DebugUtilsMessengerEXT)> {
    let debug_utils_loader = ash::extensions::ext::DebugUtils::new(entry, instance);

    if !validation::ENABLED {
        return Ok((debug_utils_loader, ash::vk::DebugUtilsMessengerEXT::null()));
    } else {
        let messenger_ci = debug_create_info()?;

        let utils_messenger = unsafe {
            debug_utils_loader
                .create_debug_utils_messenger(&messenger_ci, None)
                .expect("Debug Utils Callback")
        };
        Ok((debug_utils_loader, utils_messenger))
    }
}

unsafe extern "system" fn debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _p_user_data: *mut c_void,
) -> vk::Bool32 {
    let severity = match message_severity {
        vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE => "[Verbose]",
        vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => "[Warning]",
        vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => "[Error]",
        vk::DebugUtilsMessageSeverityFlagsEXT::INFO => "[Info]",
        _ => "[Unknown]",
    };
    let types = match message_type {
        vk::DebugUtilsMessageTypeFlagsEXT::GENERAL => "[General]",
        vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE => "[Performance]",
        vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION => "[Validation]",
        _ => "[Unknown]",
    };
    let message = CStr::from_ptr((*p_callback_data).p_message);
    println!("[Debug]{}{}{:?}", severity, types, message);

    vk::FALSE
}

fn debug_create_info() -> Result<DebugUtilsMessengerCreateInfoEXT> {
    Ok(vk::DebugUtilsMessengerCreateInfoEXT {
        s_type: vk::StructureType::DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT,
        p_next: ptr::null(),
        flags: vk::DebugUtilsMessengerCreateFlagsEXT::empty(),
        message_severity: DebugUtilsMessageSeverityFlagsEXT::WARNING
            | DebugUtilsMessageSeverityFlagsEXT::ERROR
            | DebugUtilsMessageSeverityFlagsEXT::VERBOSE,

        message_type: DebugUtilsMessageTypeFlagsEXT::GENERAL
            | DebugUtilsMessageTypeFlagsEXT::PERFORMANCE
            | DebugUtilsMessageTypeFlagsEXT::VALIDATION,
        pfn_user_callback: Some(debug_callback),
        p_user_data: ptr::null_mut(),
    })
}
