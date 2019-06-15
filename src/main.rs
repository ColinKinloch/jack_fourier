extern crate dft;
extern crate easyjack as jack;
extern crate nix;
extern crate gio;
extern crate gtk;
extern crate cairo;

use gio::prelude::*;
use gtk::prelude::*;

use dft::{Operation, Plan, c32};
use std::sync::{Arc, Mutex};
use std::cell::RefCell;

use std::f32;

use std::collections::VecDeque;
use std::sync::atomic;

const F_SIZE: usize = 512;
const HEIGHT: usize = 720;
const H_SCALE: f64 = 2.;
const F_SCALE: f64 = 3.;

type InputPort  = jack::InputPortHandle<jack::DefaultAudioSample>;

struct Connector {
  input: InputPort,
  samples: Arc<Mutex<VecDeque<f32>>>,
}

impl Connector {
  pub fn new(input: InputPort, samples: Arc<Mutex<VecDeque<f32>>>) -> Self {
    Connector {
      input,
      samples,
    }
  }
}

impl jack::ProcessHandler for Connector {
  fn process(&mut self, ctx: &jack::CallbackContext, nframes: jack::NumFrames) -> i32 {
    let i = self.input.get_read_buffer(nframes, ctx);
    self.samples.lock().unwrap().extend(i.iter());

    // return 0 so jack lets us keep running
    0
  }
}

fn as_c32_mut(slice: &mut [f32]) -> &mut [c32] {
  unsafe { std::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut _, slice.len() / 2) }
}

fn interp_colours(colours: &[[f32; 3]], value: f32) -> Vec<f32> {
  let l = colours.len() as f32;
  let mut bv = (l - 1.) * value;
  bv = bv.min(l - 1.);//if bv > l {bv = l};
  let d = (bv - bv.floor()) * bv.ceil();
  let ref low_colour = colours[bv.floor() as usize];
  let ref high_colour = colours[bv.ceil() as usize];
  low_colour.iter().zip(high_colour.iter()).map(|(a, b)| a * (1. - d) + b * d).collect::<Vec<f32>>()
  //low_colour.clone()
}

fn build_ui(application: &gtk::Application, running: Arc<atomic::AtomicBool>, frequencies: Arc<Mutex<VecDeque<f32>>>) {
  let window = gtk::ApplicationWindow::new(application);

  window.set_title("Jack Fourier");
  window.set_border_width(10);
  window.set_position(gtk::WindowPosition::Center);
  window.set_default_size((F_SIZE as f64) as i32, HEIGHT as i32);

  let drawing_area = gtk::DrawingArea::new();
  //drawing_area.set_size_request(F_SIZE as i32, HEIGHT as i32);
  
  let gain_adjustment = gtk::Adjustment::new(1.0, 0.0, 2.0, 0.01, 0.01, 0.1);
  {
    let raster = RefCell::new(cairo::ImageSurface::create(cairo::Format::Rgb24, F_SIZE as i32, HEIGHT as i32).unwrap());
    let counter = RefCell::new(0);
    let colour_set = [
      [0.0, 0.0, 0.0], // Black
      [1.0, 0.0, 0.0], // Blue
      [1.0, 1.0, 0.0], // 
      [0.0, 1.0, 1.0], // 
      [0.0, 0.0, 1.0], // Blue
      [1.0, 1.0, 1.0]
    ];
    let gain_adjustment = gain_adjustment.clone();
    drawing_area.connect_draw(move |drawing_area, cr| {
      let da_alloc = drawing_area.get_allocation();
      let scale_height = (f64::from(da_alloc.height) / H_SCALE) as usize;
      {
        let mut raster = raster.borrow_mut();
        let stride = raster.get_stride() as usize;
        let height = raster.get_height() as usize;
        let width = raster.get_width() as usize;
        let mut d = raster.get_data().unwrap();

        // let mut data = d.deref_mut();
        
        let mut frequencies = frequencies.lock().unwrap();
        
        //let plan = plan.borrow();
        while frequencies.len() > F_SIZE {
          let mut counter = counter.borrow_mut();
          *counter += 1;
          
          let mut m = 0.0_f32;
          const BBP: usize = 4;
          for (i, s) in (0..width).zip(frequencies.drain(0..F_SIZE)) {
            //for j in 0..(height/2) {
            {
              let j = *counter % scale_height.min(height) as usize;
              let p = stride*j;
              let g = s.abs() * F_SIZE as f32;
              m = m.max(g);
              //println!("{} = {}", s, g);
              let mut ic = interp_colours(&colour_set, (g * gain_adjustment.get_value() as f32).sqrt());
              // [B, G, R]
              //let mut ic = interp_colours(&[[0.0, 1.0, 1.0]], g.sqrt());
              //let g = g.sqrt() * 255.;
              for c in ic.iter_mut() { *c *= 255. };
              let iic = ic.iter().map(|v| if *v > 255. {255} else {*v as u8}).collect::<Vec<_>>();
              for (j, c) in iic.iter().enumerate() {
                d[p+i*BBP+j] = *c;
              }
              d[p+i*BBP+3] = 0;
            // d[BBP*j+2] = 0;
            }
          }
          //println!("{}", m);
        }
      }
      cr.scale(F_SCALE, H_SCALE);
      
      //raster.borrow().set_device_scale(5.0, 1.0);
      cr.set_source_surface(&*raster.borrow(), 0., 0.);
      //cr.set_source_rgba(1.0, 0.0, 1.0, 1.0);
      // cr.set_line_width(100.0);
      cr.rectangle(0.0, 0.0, F_SIZE as f64, HEIGHT as f64);
      cr.fill();
      Inhibit(false)
    });
  }
  {
    let drawing_area = drawing_area.clone();
    idle_add(move || {
      drawing_area.queue_draw();
      Continue(running.load(atomic::Ordering::SeqCst))
    });
  }

  window.add(&drawing_area);

  let header_bar = gtk::HeaderBar::new();
  header_bar.set_show_close_button(true);
  header_bar.set_title(Some(window.get_title().unwrap().as_str()));

  let menu_button = gtk::MenuButton::new();

  let popover = gtk::Popover::new(&window);

  let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 3);


  let gain_scale = gtk::Scale::new(gtk::Orientation::Horizontal, &gain_adjustment);

  gain_scale.set_digits(3);

  menu_box.add(&gain_scale);
  menu_box.set_property_width_request(500);
  menu_box.show_all();

  popover.add(&menu_box);

  menu_button.set_popover(&popover);

  header_bar.pack_end(&menu_button);

  window.set_titlebar(&header_bar);

  window.show_all();
}

fn main() {
  let running = Arc::new(atomic::AtomicBool::new(true));

  let mut jack_client =
      jack::Client::open("jack_fourier", jack::options::NO_START_SERVER).unwrap().0;

  println!("client created named: {}", jack_client.get_name());

  let input = jack_client.register_input_audio_port("input").unwrap();

  let samples = Arc::new(Mutex::new(VecDeque::new()));
  let frequencies = Arc::new(Mutex::new(VecDeque::new()));

  {
    use std::thread;
    use std::time::Duration;
    let running = running.clone();
    let samples = samples.clone();
    let frequencies = frequencies.clone();
    thread::spawn(move || {
      let plan = Plan::new(Operation::Inverse, F_SIZE);
      while running.load(atomic::Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(25));
        let mut samples = samples.lock().unwrap();
        let mut frequencies = frequencies.lock().unwrap();
        while samples.len() > F_SIZE {
          let mut data = samples.drain(0..F_SIZE).collect::<Vec<_>>();
          //let data = s.collect::<Vec<_>>(); // vec![0.0; 32];
          //let mut data = vec![0.0; i.len()];
          // data.clone_from_slice(&i[0..32]);
          dft::transform(as_c32_mut(&mut data), &plan);
          frequencies.extend(data.iter());
        }
      }
    });
  }

  let handler = Connector::new(input, samples.clone());
  jack_client.set_process_handler(handler).unwrap();
  
  let application = gtk::Application::new("org.kinloch.colin.jack_fourier",
                                          gio::ApplicationFlags::NON_UNIQUE)
                                     .expect("Initialization failed...");


  // start everything up
  jack_client.activate().unwrap();

  let jack_client = Arc::new(Mutex::new(jack_client));
  {
    let running = running.clone();
    let frequencies = frequencies.clone();
    application.connect_activate(move |app| {
      build_ui(app, running.clone(), frequencies.clone());
    });
  }
  {
    let running = running.clone();
    application.connect_shutdown(move |_app| {
      running.store(false, atomic::Ordering::SeqCst);
      jack_client.lock().unwrap().close().unwrap();
    });
  }

  application.run(&std::env::args().collect::<Vec<_>>());
}
