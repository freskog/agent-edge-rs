// Minimal test case for debugging melspectrogram model issue with GDB
use tflitec::interpreter::Options;
use tflitec::tensor::Shape;
use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

#[cfg(test)]
#[test]
fn debug_melspec_model_minimal() {
    println!("=== DEBUGGING MELSPECTROGRAM MODEL ===");

    // Step 1: Load model
    println!("Step 1: Loading melspectrogram model...");
    let model = TfliteModel::new("models/melspectrogram.tflite").unwrap();
    println!("✅ Model loaded successfully");

    // Step 2: Create interpreter
    println!("Step 2: Creating interpreter...");
    let options = Options::default();
    let interpreter = Interpreter::new(&model, Some(options)).unwrap();
    println!("✅ Interpreter created successfully");

    // Step 3: First try to allocate tensors with default shape
    println!("Step 3: Allocating tensors with default shape...");
    let default_allocate_result = interpreter.allocate_tensors();
    match default_allocate_result {
        Ok(()) => {
            println!("✅ Default tensors allocated successfully");

            // Check input tensor info with default shape
            let input_tensor = interpreter.input(0).unwrap();
            let input_shape = input_tensor.shape();
            println!("  Default input shape: {:?}", input_shape.dimensions());
            println!(
                "  Default input elements: {}",
                input_shape.dimensions().iter().product::<usize>()
            );
        }
        Err(e) => {
            println!("❌ Default tensor allocation failed: {}", e);
        }
    }

    // Step 4: Try to resize input tensor (this is where Python does resize_tensor_input)
    println!("Step 4: Resizing input tensor to [1, 1280]...");
    let new_shape = Shape::new(vec![1, 1280]);
    let resize_result = interpreter.resize_input(0, new_shape);
    match resize_result {
        Ok(()) => println!("✅ Input tensor resized successfully"),
        Err(e) => {
            println!("❌ Input tensor resize failed: {}", e);
            return;
        }
    }

    // Step 5: Try to allocate tensors after resize (this is where it fails)
    println!("Step 5: Allocating tensors after resize...");
    println!("  >>> THIS IS WHERE THE OVERFLOW HAPPENS <<<");
    let allocate_result = interpreter.allocate_tensors();
    match allocate_result {
        Ok(()) => {
            println!("✅ Tensors allocated successfully");

            // Check tensor info after successful allocation
            let input_tensor = interpreter.input(0).unwrap();
            let input_shape = input_tensor.shape();
            println!("  New input shape: {:?}", input_shape.dimensions());
            println!(
                "  New input elements: {}",
                input_shape.dimensions().iter().product::<usize>()
            );
        }
        Err(e) => println!("❌ Tensor allocation failed: {}", e),
    }
}

fn main() {
    debug_melspec_model_minimal();
}
