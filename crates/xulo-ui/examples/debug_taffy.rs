//! Debug: check Taffy style generation.

use taffy::prelude::TaffyAuto;
use xulo_ui::{Dimension, StyleProps};

fn main() {
    // Test style with height: 10
    let style = StyleProps {
        height: Some(Dimension::Pixels(10)),
        ..StyleProps::default()
    };

    println!("Input StyleProps: {:?}", style);

    // Convert to Taffy style (inline the conversion logic)
    let size_height = style
        .height
        .map(|d| match d {
            Dimension::Pixels(n) => {
                println!(
                    "Dimension::Pixels({}) -> taffy::Dimension::length({})",
                    n, n
                );
                taffy::Dimension::length(n as f32)
            }
            Dimension::Full => {
                println!("Dimension::Full -> taffy::Dimension::percent(1.0)");
                taffy::Dimension::percent(1.0)
            }
        })
        .unwrap_or({
            println!("None -> taffy::Dimension::AUTO");
            taffy::Dimension::AUTO
        });

    println!("Result size.height: {:?}", size_height);
    println!("Expected: taffy::Dimension::length(10.0)");
}
