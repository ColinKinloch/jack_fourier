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
use nix::sys::signal;
use std::sync::atomic;

const F_SIZE: usize = 512;
const HEIGHT: usize = 1080;
const H_SCALE: f64 = 2.;
const F_SCALE: f64 = 3.;

// signals are unpleasant (check comments in simple_client example)
static RUNNING: atomic::AtomicBool = atomic::AtomicBool::new(true);

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


extern "C" fn handle_sigint(_: i32) {
    RUNNING.store(false, atomic::Ordering::SeqCst);
}

fn build_ui(application: &gtk::Application, samples: Arc<Mutex<VecDeque<f32>>>) {
    let window = gtk::ApplicationWindow::new(application);

    window.set_title("Jack Fourier");
    window.set_border_width(10);
    window.set_position(gtk::WindowPosition::Center);
    window.set_default_size((F_SIZE as f64) as i32, HEIGHT as i32);

    let drawing_area = gtk::DrawingArea::new();
    //drawing_area.set_size_request(F_SIZE as i32, HEIGHT as i32);
    
    {
      let raster = RefCell::new(cairo::ImageSurface::create(cairo::Format::Rgb24, F_SIZE as i32, HEIGHT as i32).unwrap());
      let counter = RefCell::new(0);
      let plan = RefCell::new(Plan::new(Operation::Inverse, F_SIZE));
      drawing_area.connect_draw(move |drawing_area, cr| {
        let da_alloc = drawing_area.get_allocation();
        let scale_height = (f64::from(da_alloc.height) / H_SCALE) as usize;
        {
          let mut raster = raster.borrow_mut();
          let stride = raster.get_stride() as usize;
          // let height = raster.get_height() as usize;
          let width = raster.get_width() as usize;
          let mut d = raster.get_data().unwrap();

          // let mut data = d.deref_mut();
          
          let mut samples = samples.lock().unwrap();
          
          let plan = plan.borrow();
          while samples.len() > F_SIZE {
            let s = samples.drain(0..F_SIZE);
            let mut data = s.collect::<Vec<_>>(); // vec![0.0; 32];
            //let mut data = vec![0.0; i.len()];
            // data.clone_from_slice(&i[0..32]);
            dft::transform(as_c32_mut(&mut data), &plan);
            
            let mut counter = counter.borrow_mut();
            *counter += 1;
            
            let mut m = 0.0_f32;
            const BBP: usize = 4;
            for (i, &s) in (0..(width/2)).zip(data.iter()) {
              //for j in 0..(height/2) {
              {
                let j = *counter % scale_height as usize;
                let p = stride*j;
                let g = s.abs() * F_SIZE as f32;
                m = m.max(g);
                //println!("{} = {}", s, g);
                let g = g.sqrt() * 255.;
                let g = if g > 255. {255} else {g as u8};
                d[p+i*BBP+0] = g;
                d[p+i*BBP+1] = g;
                d[p+i*BBP+2] = g;
                d[p+i*BBP+3] = 0;
              // d[BBP*j+2] = 0;
              }
            }
            // println!("{} = {}", m, (m * 255.) as u8);
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
        Continue(RUNNING.load(atomic::Ordering::SeqCst))
      });
    }

    window.add(&drawing_area);

    window.show_all();
}

fn main() {
    // register a signal handler (see comments at top of file)
    let action = signal::SigAction::new(
        signal::SigHandler::Handler(handle_sigint),
        signal::SaFlags::empty(),
        signal::SigSet::empty());

    unsafe { signal::sigaction(signal::Signal::SIGINT, &action) }.unwrap();

    // set our global atomic to true
    RUNNING.store(true, atomic::Ordering::SeqCst);

    let mut jack_client =
        jack::Client::open("jack_fourier", jack::options::NO_START_SERVER).unwrap().0;

    println!("client created named: {}", jack_client.get_name());

    let input = jack_client.register_input_audio_port("input").unwrap();

    let samples = Arc::new(Mutex::new(VecDeque::new()));

    let handler = Connector::new(input, samples.clone());
    jack_client.set_process_handler(handler).unwrap();
    
    let application = gtk::Application::new("org.kinloch.colin.jack_fourier",
                                            Default::default())
                                       .expect("Initialization failed...");

  
    // start everything up
    jack_client.activate().unwrap();

    application.connect_activate(move |app| {
      build_ui(app, samples.clone());
    });
    application.connect_shutdown(move |_app| {
      RUNNING.store(false, atomic::Ordering::SeqCst);
    });

    application.run(&std::env::args().collect::<Vec<_>>());
    
    // now we can clean everything up
    // the library doesn't handle this for us because it would be rather confusing, especially
    // given how the underlying jack api actually works
    println!("tearing down");

    // closing the client unregisters all of the ports
    // unregistering the ports after the client is closed is an error
    jack_client.close().unwrap();
}
