use crate::handle::AssetHandle;
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fs,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

pub type DynAsset = Box<dyn Asset>;

pub trait Asset: Any {
    fn deserialize(&mut self, _path: &Path) {
        println!("[WARN]: trying to deserialize asset without implementing trait")
    }
    fn serialize(&mut self, _path: &Path) {
        println!("[WARN]: trying to serialize asset without implementing trait")
    }
}
pub trait ReloadableAsset: Asset {
    // replace with
    fn load(path: &Path) -> Self;
    fn write(&self, path: &Path);
}

impl<T: ReloadableAsset> Asset for T {
    fn deserialize(&mut self, path: &Path) {
        *self = T::load(path);
    }
    fn serialize(&mut self, path: &Path) {
        self.write(path);
    }
}

pub trait RenderAsset: Any {} // might be able to remove and enforce with convert function
pub trait ConvertableRenderAsset: RenderAsset {
    type SourceAsset: Asset;
    type Params;

    fn convert(source: &Self::SourceAsset, params: &Self::Params) -> Self;
}

impl dyn Asset {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
impl dyn RenderAsset {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Assets {
    cache: HashMap<AssetHandle<DynAsset>, DynAsset>,
    render_cache: HashMap<AssetHandle<DynAsset>, Box<dyn RenderAsset>>,

    dirty: HashSet<AssetHandle<DynAsset>>,

    serialize_handles: HashMap<AssetHandle<DynAsset>, PathBuf>,
    deserialize_handles: HashMap<PathBuf, AssetHandle<DynAsset>>,
    reload_watcher: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::FsEventWatcher>,
    reload_receiver: mpsc::Receiver<PathBuf>,
    #[allow(dead_code)]
    reload_sender: mpsc::Sender<PathBuf>,
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
            render_cache: HashMap::new(),
            dirty: HashSet::new(),
            deserialize_handles: HashMap::new(),
            serialize_handles: HashMap::new(),
            reload_receiver,
            reload_sender,
            reload_watcher,
        }
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
        // invalidate gpu cache
        self.render_cache.remove(&handle.clone().to_handle());

        // set dirty
        self.dirty.insert(handle.clone().to_handle());
        println!("set dirty {:?}", handle.id);

        // get value and convert to T
        self.cache
            .get_mut(&handle.to_handle())
            .and_then(|a| a.as_any_mut().downcast_mut::<T>())
            .unwrap()
    }

    pub fn load_from_disk<T: ReloadableAsset + 'static>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let data = T::load(&path);
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().to_handle(), Box::new(data));

        if watch {
            self.reload_watcher
                .watcher()
                .watch(
                    &path,
                    notify_debouncer_mini::notify::RecursiveMode::Recursive,
                )
                .unwrap();

            self.deserialize_handles
                .insert(path.clone(), handle.clone().to_handle());
        }

        if write {
            self.serialize_handles
                .insert(handle.clone().to_handle(), path.clone());
        }

        handle
    }

    // pub fn watch<T: ReloadableAsset + 'static>(&mut self, path: &Path) -> AssetHandle<T> {
    //     let path = fs::canonicalize(path).unwrap();
    //
    //     let handle = self.load_from_disk(&path, false, false);
    //
    //     self.reload_watcher
    //         .watcher()
    //         .watch(
    //             &path,
    //             notify_debouncer_mini::notify::RecursiveMode::Recursive,
    //         )
    //         .unwrap();
    //
    //     self.deserialize_handles
    //         .insert(path.clone(), handle.clone().to_handle());
    //     self.serialize_handles
    //         .insert(handle.clone().to_handle(), path.clone());
    //
    //     println!("set H2P {:?} to {:?}", handle.id, path);
    //
    //     handle
    // }

    pub fn convert<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
        params: &G::Params,
    ) -> &G {
        // create new if not in cache
        if !self.render_cache.contains_key(&handle.clone().to_handle()) {
            let cpu = self.get(handle.clone());
            let converted = G::convert(cpu, params);
            self.render_cache
                .insert(handle.clone().to_handle(), Box::new(converted));
        }

        // get value and convert to G
        self.render_cache
            .get(&handle.to_handle())
            .and_then(|a| a.as_any().downcast_ref::<G>())
            .unwrap()
    }

    pub fn convert_mut<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
        params: &G::Params,
    ) -> &mut G {
        // create new if not in cache
        if !self.render_cache.contains_key(&handle.clone().to_handle()) {
            let cpu = self.get_mut(handle.clone());
            let converted = G::convert(cpu, params);
            self.render_cache
                .insert(handle.clone().to_handle(), Box::new(converted));
        }

        // get value and convert to G
        self.render_cache
            .get_mut(&handle.to_handle())
            .and_then(|a| a.as_any_mut().downcast_mut::<G>())
            .unwrap()
    }

    pub fn poll_serialize(&mut self) {
        for handle in self.dirty.drain() {
            if let Some(path) = self.serialize_handles.get(&handle) {
                self.cache.get_mut(&handle).unwrap().serialize(path);
            }
        }
    }
    pub fn poll_deserialize(&mut self) {
        for path in self.reload_receiver.try_iter() {
            let handle = self.deserialize_handles.get_mut(&path).unwrap();

            // reload
            self.cache.get_mut(handle).unwrap().deserialize(&path);

            // invalidate potential gpu cache
            self.render_cache.remove(handle);
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
