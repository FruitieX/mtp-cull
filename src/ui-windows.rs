/*!
    A application that uses the `image-decoder` feature to load resources and display them.

    Requires the following features: `cargo run --example image_decoder_d --features "image-decoder file-dialog"`
*/

extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

use nwd::NwgUi;
use nwg::NativeUi;
use nwg::{
    stretch::{
        geometry::{Rect, Size},
        style::{AlignSelf, Dimension as D, FlexDirection},
    },
    EventData,
};
use std::cell::RefCell;
use std::env;

#[derive(Default, NwgUi)]
pub struct ImageDecoderApp {
    // The image that will be loaded dynamically
    loaded_image: RefCell<Option<nwg::Bitmap>>,

    #[nwg_control(size: (1280, 720), center: true, maximized: true, title: "Image decoder")]
    #[nwg_events( OnWindowClose: [ImageDecoderApp::exit], OnSysKeyPress: [ImageDecoderApp::on_key_down(SELF, EVT_DATA)], OnKeyPress: [ImageDecoderApp::on_key_down(SELF, EVT_DATA)], OnKeyEnter: [ImageDecoderApp::on_key_down(SELF, EVT_DATA)] )]
    window: nwg::Window,

    #[nwg_layout(parent: window)]
    main_layout: nwg::GridLayout,

    #[nwg_resource]
    decoder: nwg::ImageDecoder,

    #[nwg_resource(title: "Open File", action: nwg::FileDialogAction::Open, filters: "Jpeg(*.jpg;*.jpeg)|Png(*.png)|DDS(*.dds)|TIFF(*.tiff)|BMP(*.bmp)|Any (*.*)")]
    dialog: nwg::FileDialog,

    #[nwg_control]
    #[nwg_layout_item(layout: main_layout, col: 0, row: 0, row_span: 4, col_span: 5)]
    img: nwg::ImageFrame,

    #[nwg_control(text: "Open", size: (200, 100))]
    #[nwg_layout_item(layout: main_layout, col: 5, row: 0)]
    #[nwg_events(OnButtonClick: [ImageDecoderApp::open_file], OnSysKeyPress: [ImageDecoderApp::on_key_down(SELF, EVT_DATA)], OnKeyPress: [ImageDecoderApp::on_key_down(SELF, EVT_DATA)], OnKeyEnter: [ImageDecoderApp::on_key_down(SELF, EVT_DATA)] )]
    open_btn: nwg::Button,

    #[nwg_control(size: (200, 100))]
    #[nwg_layout_item(layout: main_layout, col: 5, row: 1 )]
    file_name: nwg::Label,
}

impl ImageDecoderApp {
    fn open_file(&self) {
        if let Ok(d) = env::current_dir() {
            if let Some(d) = d.to_str() {
                self.dialog
                    .set_default_folder(d)
                    .expect("Failed to set default folder.");
            }
        }

        if self.dialog.run(Some(&self.window)) {
            self.file_name.set_text("");
            if let Ok(directory) = self.dialog.get_selected_item() {
                let dir = directory.into_string().unwrap();
                self.file_name.set_text(&dir);
                self.read_file();
            }
        }
    }

    fn read_file(&self) {
        println!("{}", self.file_name.text());
        let image = match self.decoder.from_filename(&self.file_name.text()) {
            Ok(img) => img,
            Err(_) => {
                println!("Could not read image!");
                return;
            }
        };

        println!("Frame count: {}", image.frame_count());
        println!("Format: {:?}", image.container_format());

        let frame = match image.frame(0) {
            Ok(bmp) => bmp,
            Err(_) => {
                println!("Could not read image frame!");
                return;
            }
        };

        let size = frame.size();
        println!("Size: {:?}", size);

        let aspect = size.0 as f32 / size.1 as f32;
        let container_size = self.img.size();
        let x = container_size.0 as f32;
        let y = container_size.1 as f32;
        let image_size = if x / y > aspect {
            Size {
                width: y * aspect,
                height: y,
            }
        } else {
            Size {
                width: x,
                height: x / aspect,
            }
        };
        let resized = self
            .decoder
            .resize_image(&frame, [image_size.width as u32, image_size.height as u32])
            .unwrap();

        // Create a new Bitmap image from the image data
        match resized.as_bitmap() {
            Ok(bitmap) => {
                let mut img = self.loaded_image.borrow_mut();
                img.replace(bitmap);
                self.img.set_bitmap(img.as_ref());
            }
            Err(_) => {
                println!("Could not convert image to bitmap!");
            }
        }
    }

    fn exit(&self) {
        nwg::stop_thread_dispatch();
    }

    fn on_key_down(&self, key: &EventData) {
        dbg!(key);
    }
}

pub fn init() {
    nwg::init().expect("Failed to init Native Windows GUI");
    nwg::Font::set_global_family("Segoe UI").expect("Failed to set default font");

    let _app = ImageDecoderApp::build_ui(Default::default()).expect("Failed to build UI");

    nwg::dispatch_thread_events();
}
