use assets::{Asset, Assets, ConvertableRenderAsset, GpuAsset, ReloadableAsset};
use std::{fs::read_to_string, os::unix::thread, path::Path, thread::sleep, time::Duration};

mod assets;
mod handle;

fn main() {
    let mut assets = Assets::new();

    let person1 = assets.insert(Person {
        name: String::from("bro"),
        age: 12,
    });
    let person2 = assets.load_from_disk::<Person>(Path::new("assets/alice.person"));
    let person3 = assets.watch::<Person>(Path::new("assets/bob.person"));
    let house1 = assets.watch::<House>(Path::new("assets/house1"));
    let shader = assets.watch::<Shader>(Path::new("assets/shader"));

    assets.get_mut(person1.clone());

    loop {
        sleep(Duration::from_millis(1000));

        println!("shader: {:?}", assets.get(shader.clone()));
        println!(
            "shader gpu: {:?}",
            assets.convert_mut::<GpuShader>(shader.clone())
        );
        let gpu_shader = assets.convert(shader.clone());
        print_gpu_shader(gpu_shader);

        assets.poll_reload();
    }
}

fn print_gpu_shader(shader: &GpuShader) {}

#[derive(Debug)]
struct Shader {
    source: String,
}

impl ReloadableAsset for Shader {
    fn load(path: &Path) -> Self {
        println!("reload shader");
        let content = read_to_string(path).unwrap();
        Self { source: content }
    }
}

#[derive(Debug)]
struct GpuShader {
    module: u32, // handle
}

impl GpuAsset for GpuShader {}
impl ConvertableRenderAsset for GpuShader {
    type SourceAsset = Shader;

    fn convert(source: &Self::SourceAsset) -> Self {
        println!("convert shader to gpu shader");
        let id = source.source.trim().parse::<u32>().unwrap();
        Self { module: id }
    }
}

#[derive(Debug)]
struct House {
    price: u32,
}

impl ReloadableAsset for House {
    fn load(path: &Path) -> Self {
        let inp = read_to_string(path).unwrap();
        let mut split = inp.split_whitespace();
        let price = split.next().unwrap();
        println!("{price}");
        let price = price.parse::<u32>().unwrap();
        Self { price }
    }
}

#[derive(Debug)]
struct Person {
    name: String,
    age: u32,
}

impl ReloadableAsset for Person {
    fn load(path: &Path) -> Self {
        let inp = read_to_string(path).unwrap();
        let mut split = inp.split_whitespace();
        let name = split.next().unwrap().to_string();
        let age = split.next().unwrap().parse::<u32>().unwrap();
        Self { name, age }
    }
}
