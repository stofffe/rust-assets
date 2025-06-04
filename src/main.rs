use assets::{
    ArcHandle, Asset, Assets, ConvertableRenderAsset, LoadableAsset, RenderAsset, WriteableAsset,
};
use std::{fmt::Write, fs::read_to_string, path::Path, thread::sleep, time::Duration};

mod assets;
mod handle;

fn main() {
    let mut assets = Assets::new();

    let person1 = assets.insert(Person {
        name: String::from("bro"),
        age: 12,
    });
    let person2 = assets.load_async::<Person>(Path::new("assets/alice.person"), true, true);
    let shader = assets.load_async::<Shader>(Path::new("assets/shader"), true, false);

    let mut i = 0;
    loop {
        sleep(Duration::from_millis(1000));
        if i % 5 == 0 {
            // update manually
            if let Some(person) = assets.get_mut(person2.clone()) {
                person.age += 1;
            }
        }

        println!("shader: {:?}", assets.get(shader.clone()));
        let gpu_shader = assets.convert(shader.clone(), &100);
        if let Some(gpu_shader) = gpu_shader {
            print_gpu_shader(gpu_shader);
        }

        if let Some(person) = assets.get(person2.clone()) {
            println!("person {:?}", person);
        } else {
            println!("person not loaded");
        }

        assets.poll_reload();
        assets.poll_write();
        assets.poll_loaded();

        i += 1;
    }
}

fn print_gpu_shader(shader: ArcHandle<GpuShader>) {
    println!("{:?}", shader)
}

#[derive(Debug)]
struct Person {
    name: String,
    age: u32,
}

impl Asset for Person {}
impl LoadableAsset for Person {
    fn load(path: &Path) -> Self {
        let inp = read_to_string(path).unwrap();
        let mut split = inp.split_whitespace();
        let name = split.next().unwrap().to_string();
        let age = split.next().unwrap().parse::<u32>().unwrap();
        Self { name, age }
    }
}
impl WriteableAsset for Person {
    fn write(&mut self, path: &Path) {
        let mut output = String::new();
        output.write_str(&self.name).unwrap();
        output.write_char(' ').unwrap();
        output.write_str(&self.age.to_string()).unwrap();
        std::fs::write(path, output).expect("could not write to person");
    }
}

#[derive(Debug)]
struct Shader {
    source: String,
}

impl Asset for Shader {}
impl LoadableAsset for Shader {
    fn load(path: &Path) -> Self {
        let content = read_to_string(path).expect("could not read shader from disk");
        Self { source: content }
    }
}
impl WriteableAsset for Shader {
    fn write(&mut self, path: &Path) {
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
