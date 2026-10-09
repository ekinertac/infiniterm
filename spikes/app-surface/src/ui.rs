// The probe's whole UI: a counter and a text field. It is the same function
// in both modes (a standalone window and a surface drawn into a card), so a
// difference between the two cannot come from the widgets.

#[derive(Default)]
pub struct Probe {
    pub count: i64,
    pub text: String,
}

impl Probe {
    pub fn ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("app-surface probe");
            ui.horizontal(|ui| {
                if ui.button("-").clicked() {
                    self.count -= 1;
                }
                ui.label(self.count.to_string());
                if ui.button("+").clicked() {
                    self.count += 1;
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.text)
                    .hint_text("type here, dead keys and emoji too")
                    .desired_width(f32::INFINITY),
            );
            ui.label(format!("{} chars", self.text.chars().count()));
        });
    }
}
