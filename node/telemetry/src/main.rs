use color_eyre::Result;

pub fn main() -> Result<()> {
    color_eyre::install()?;
    println!("telemetry module");
    Ok(())
}