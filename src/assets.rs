use crate::handle::AssetHandle;
use std::{
    any::Any,
    collections::HashMap,
    convert, fs,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

pub type DynAsset = Box<dyn Asset>;

pub trait Asset: Any {
    fn reload(&mut self, _path: &Path) {
        println!("[WARN]: trying to reload asset without implementing reload trait function")
    }
}
pub trait ReloadableAsset: Asset {
    fn load(path: &Path) -> Self;
}

impl<T: ReloadableAsset> Asset for T {
    fn reload(&mut self, path: &Path) {
        *self = T::load(path);
    }
}

pub trait GpuAsset: Any {}
pub trait ConvertableRenderAsset: GpuAsset {
    type SourceAsset: Asset;

    fn convert(source: &Self::SourceAsset) -> Self;
}

impl dyn Asset {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
impl dyn GpuAsset {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Assets {
    cache: HashMap<AssetHandle<DynAsset>, DynAsset>,

    reload_paths: HashMap<PathBuf, AssetHandle<DynAsset>>,
    reload_watcher: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::FsEventWatcher>,
    reload_receiver: mpsc::Receiver<PathBuf>,
    reload_sender: mpsc::Sender<PathBuf>,

    gpu: HashMap<AssetHandle<DynAsset>, Box<dyn GpuAsset>>,
}

impl Assets {
    pub fn new() -> Self {
        let (reload_sender, reload_receiver) = mpsc::channel();
        let sender_copy = reload_sender.clone();

        let reload_watcher = notify_debouncer_mini::new_debouncer(
            Duration::from_millis(100),
            move |res: notify_debouncer_mini::DebounceEventResult| match res {
                Ok(events) => {
                    for event in events {
                        sender_copy
                            .clone()
                            .send(event.path)
                            .expect("could not send");
                    }
                }
                Err(e) => println!("Error {:?}", e),
            },
        )
        .expect("could not create watcher");

        Self {
            cache: HashMap::new(),
            gpu: HashMap::new(),
            reload_paths: HashMap::new(),
            reload_receiver,
            reload_sender,
            reload_watcher,
        }
    }

    pub fn convert<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
    ) -> &G {
        // create new if not in cache
        if !self.gpu.contains_key(&handle.clone().to_handle()) {
            let cpu = self.get(handle.clone());
            let converted = G::convert(cpu);
            self.gpu
                .insert(handle.clone().to_handle(), Box::new(converted));
        }

        // get value and convert to G
        self.gpu
            .get(&handle.to_handle())
            .and_then(|a| a.as_any().downcast_ref::<G>())
            .unwrap()
    }
    pub fn convert_mut<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
    ) -> &mut G {
        // create new if not in cache
        if !self.gpu.contains_key(&handle.clone().to_handle()) {
            let cpu = self.get_mut(handle.clone());
            let converted = G::convert(cpu);
            self.gpu
                .insert(handle.clone().to_handle(), Box::new(converted));
        }

        // get value and convert to G
        self.gpu
            .get_mut(&handle.to_handle())
            .and_then(|a| a.as_any_mut().downcast_mut::<G>())
            .unwrap()
    }

    pub fn insert<T: Asset + 'static>(&mut self, data: T) -> AssetHandle<T> {
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().to_handle(), Box::new(data));
        handle
    }

    pub fn get<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> &T {
        self.cache
            .get(&handle.to_handle())
            .and_then(|a| a.as_any().downcast_ref::<T>())
            .unwrap()
    }

    pub fn get_mut<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> &mut T {
        self.cache
            .get_mut(&handle.to_handle())
            .and_then(|a| a.as_any_mut().downcast_mut::<T>())
            .unwrap()
    }

    pub fn load_from_disk<T: ReloadableAsset + 'static>(&mut self, path: &Path) -> AssetHandle<T> {
        let data = T::load(path);
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().to_handle(), Box::new(data));
        handle
    }

    pub fn watch<T: Asset + ReloadableAsset + 'static>(&mut self, path: &Path) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let handle = self.load_from_disk(&path);

        self.reload_watcher
            .watcher()
            .watch(
                &path,
                notify_debouncer_mini::notify::RecursiveMode::Recursive,
            )
            .unwrap();

        self.reload_paths.insert(path, handle.clone().to_handle());

        handle
    }

    pub fn poll_reload(&mut self) {
        for path in self.reload_receiver.try_iter() {
            let handle = self.reload_paths.get_mut(&path).unwrap();

            // reload
            self.cache.get_mut(handle).unwrap().reload(&path);

            // invalidate potential gpu cache
            self.gpu.remove(handle);
        }
    }

    pub fn force_reload(&self, path: PathBuf) {
        self.reload_sender.send(path).expect("could not send path");
    }
}

impl<T> AssetHandle<T> {
    pub fn to_handle(self) -> AssetHandle<DynAsset> {
        AssetHandle::<DynAsset> {
            id: self.id,
            ty: PhantomData,
        }
    }
    pub fn from_handle(any_handle: AssetHandle<DynAsset>) -> Self {
        AssetHandle {
            id: any_handle.id,
            ty: PhantomData,
        }
    }
}
