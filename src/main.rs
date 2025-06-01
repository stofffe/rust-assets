use assets::{Assets, ConvertableRenderAsset, ReloadableAsset, RenderAsset};
use std::{fmt::Write, fs::read_to_string, path::Path, thread::sleep, time::Duration};

mod assets;
mod handle;

fn main() {
    let mut assets = Assets::new();

    let person1 = assets.insert(Person {
        name: String::from("bro"),
        age: 12,
    });
    let person2 = assets.load_from_disk::<Person>(Path::new("assets/alice.person"), true, true);
    let person3 = assets.load_from_disk::<Person>(Path::new("assets/bob.person"), true, false);
    let house1 = assets.load_from_disk::<House>(Path::new("assets/house1"), true, true);
    let shader = assets.load_from_disk::<Shader>(Path::new("assets/shader"), true, false);

    assets.get_mut(person1.clone());

    loop {
        sleep(Duration::from_millis(1000));

        println!("shader: {:?}", assets.get(shader.clone()));
        let gpu_shader = assets.convert(shader.clone(), &100);
        print_gpu_shader(gpu_shader);

        println!("person2: {:?}", assets.get(person2.clone()));
        assets.get_mut(person2.clone()).age += 1;
        assets.get_mut(house1.clone()).price += 1;

        assets.poll_serialize();
        assets.poll_deserialize();
    }
}

fn print_gpu_shader(shader: &GpuShader) {
    println!("{:?}", shader)
}

#[derive(Debug)]
struct Shader {
    source: String,
}

impl ReloadableAsset for Shader {
    fn load(path: &Path) -> Self {
        println!("reload shader");
        let content = read_to_string(path).expect("could not read shader from disk");
        Self { source: content }
    }

    fn write(&self, path: &Path) {
        std::fs::write(path, &self.source).expect("could not write shader to disk");
    }
}

#[derive(Debug)]
struct GpuShader {
    module: u32, // handle
}

impl RenderAsset for GpuShader {}
impl ConvertableRenderAsset for GpuShader {
    type SourceAsset = Shader;
    type Params = u32;

    fn convert(source: &Self::SourceAsset, params: &Self::Params) -> Self {
        println!("convert shader to gpu shader");
        let id = source.source.trim().parse::<u32>().unwrap();
        Self {
            module: id + *params,
        }
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

    fn write(&self, path: &Path) {}
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
    fn write(&self, path: &Path) {
        let mut output = String::new();
        output.write_str(&self.name).unwrap();
        output.write_char(' ').unwrap();
        output.write_str(&self.age.to_string()).unwrap();
        std::fs::write(path, output).expect("could not write to person");
    }
}
