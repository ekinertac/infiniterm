// app-surface: one probe app, two ways to run it.
//
//   app-surface                       a standalone window (eframe)
//   app-surface --surface <socket>    no window: draws into an IOSurface and
//                                     talks to the host over <socket>
//
// The host owns the socket and the card; see protocol.rs for the messages
// and NOTES.md for what was measured.

mod protocol;
mod surface;
mod ui;

struct Standalone {
    probe: ui::Probe,
}

impl eframe::App for Standalone {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.probe.ui(ctx);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--surface" {
        return surface::run(&args[2]);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([420.0, 260.0]),
        ..Default::default()
    };
    eframe::run_native(
        "app-surface",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Standalone {
                probe: ui::Probe::default(),
            }))
        }),
    )?;
    Ok(())
}
