use crate::handle::AssetHandle;
use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    fs,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

pub type DynAsset = Box<dyn Asset>;

pub trait Asset: Any + Send + Sync {
    fn load(path: &Path) -> Self
    where
        Self: Sized;

    fn reload(&mut self, _path: &Path) {
        println!("[WARN]: trying to deserialize asset without implementing trait")
    }
    fn write(&mut self, _path: &Path) {
        println!("[WARN]: trying to serialize asset without implementing trait")
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
type AssetLoaderFn = fn(&Path) -> Box<dyn Asset>;
pub struct Assets {
    cache: HashMap<AssetHandle<DynAsset>, Option<DynAsset>>,
    render_cache: HashMap<AssetHandle<DynAsset>, Box<dyn RenderAsset>>,

    serialize_handles: HashMap<AssetHandle<DynAsset>, PathBuf>,
    serialize_dirty: HashSet<AssetHandle<DynAsset>>,

    deserialize_handles: HashMap<PathBuf, (AssetHandle<DynAsset>, AssetLoaderFn)>,

    // reloading
    reload_watcher: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::FsEventWatcher>,
    reload_receiver: mpsc::Receiver<PathBuf>,
    reload_sender: mpsc::Sender<PathBuf>,

    // async loading
    loaded_sender: mpsc::Sender<(AssetHandle<DynAsset>, DynAsset)>,
    loaded_receiver: mpsc::Receiver<(AssetHandle<DynAsset>, DynAsset)>,
}

impl Assets {
    pub fn new() -> Self {
        let (reload_sender, reload_receiver) = mpsc::channel();
        let (loaded_sender, loaded_receiver) = mpsc::channel();
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
                Err(err) => println!("debounced result error: {}", err),
            },
        )
        .expect("could not create watcher");

        Self {
            cache: HashMap::new(),
            render_cache: HashMap::new(),
            serialize_dirty: HashSet::new(),
            deserialize_handles: HashMap::new(),
            serialize_handles: HashMap::new(),

            reload_receiver,
            reload_sender,
            reload_watcher,

            loaded_sender,
            loaded_receiver,
        }
    }

    //
    // Assets
    //

    pub fn insert<T: Asset + 'static>(&mut self, data: T) -> AssetHandle<T> {
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().to_handle(), Some(Box::new(data)));
        handle
    }

    // TODO: add get_or_default (e.g. 1x1 white pixel for image)
    //
    // could return error union [Ok, Invalid, Loading]
    pub fn get<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> Option<&T> {
        self.cache
            .get(&handle.to_handle())
            .expect("invalid handle")
            .as_ref()
            .and_then(|asset| asset.as_any().downcast_ref::<T>())
    }

    pub fn get_mut<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> Option<&mut T> {
        // invalidate gpu cache
        self.render_cache.remove(&handle.clone().to_handle());

        // set dirty
        self.serialize_dirty.insert(handle.clone().to_handle());

        // get value and convert to T
        self.cache
            .get_mut(&handle.to_handle())
            .expect("invalid handle")
            .as_mut()
            .and_then(|asset| asset.as_any_mut().downcast_mut::<T>())
    }

    //
    // Reloading
    //

    pub fn load_sync<T: Asset + 'static>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let data = T::load(&path);
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().to_handle(), Some(Box::new(data)));

        if watch {
            self.reload_watcher
                .watcher()
                .watch(
                    &path,
                    notify_debouncer_mini::notify::RecursiveMode::Recursive,
                )
                .unwrap();

            self.deserialize_handles.insert(
                path.clone(),
                (handle.clone().to_handle(), |p| Box::new(T::load(p))),
            );
        }

        if write {
            self.serialize_handles
                .insert(handle.clone().to_handle(), path.clone());
        }

        handle
    }

    pub fn load_async<T: Asset + 'static>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let handle = AssetHandle::<T>::new();
        self.cache.insert(handle.clone().to_handle(), None);

        let path_clone = path.clone();
        let handle_clone = handle.clone();
        let loaded_sender_clone = self.loaded_sender.clone();

        std::thread::spawn(move || {
            println!("start async load");
            std::thread::sleep(Duration::from_millis(2000));
            let data = T::load(&path_clone);
            loaded_sender_clone
                .send((handle_clone.to_handle(), Box::new(data)))
                .expect("could not send");
            println!("end async load");
        });

        if watch {
            self.reload_watcher
                .watcher()
                .watch(
                    &path,
                    notify_debouncer_mini::notify::RecursiveMode::Recursive,
                )
                .unwrap();

            self.deserialize_handles.insert(
                path.clone(),
                (handle.clone().to_handle(), |p| Box::new(T::load(p))),
            );
        }

        if write {
            self.serialize_handles
                .insert(handle.clone().to_handle(), path.clone());
        }

        handle
    }

    pub fn poll_write(&mut self) {
        for handle in self.serialize_dirty.drain() {
            if let Some(path) = self.serialize_handles.get(&handle) {
                let asset = self.cache.get_mut(&handle).expect("invalid handle");
                if let Some(asset) = asset {
                    asset.write(path);
                    println!("write {:?}", path);
                }
            }
        }
    }

    // checks if any files changed and spawns a thread which reloads the data
    pub fn poll_reload(&mut self) {
        for path in self.reload_receiver.try_iter() {
            let (handle, factory) = self.deserialize_handles.get_mut(&path).unwrap();

            //
            // SYNC
            //
            let asset = self.cache.get_mut(handle).expect("invalid handle");
            if let Some(asset) = asset {
                asset.reload(&path);
                println!("load {:?}", path);
            }

            //
            // ASYNC
            //

            let handle_clone = handle.clone();
            let path_clone = path.clone();
            let sender_clone = self.loaded_sender.clone();
            let factory_clone = factory.clone();
            std::thread::spawn(move || {
                // println!("reload start {:?}", path);
                std::thread::sleep(Duration::from_millis(10000));
                sender_clone
                    .send((handle_clone, factory_clone(&path_clone)))
                    .expect("could not send");
                // println!("reload end {:?}", path);
            });
        }
    }

    // check if any files completed loading and update cache and invalidate render cache
    pub fn poll_loaded(&mut self) {
        for (handle, asset) in self.loaded_receiver.try_iter() {
            self.cache.insert(handle.clone(), Some(asset));
            self.render_cache.remove(&handle);
        }
    }

    pub fn force_reload(&self, path: PathBuf) {
        self.reload_sender.send(path).expect("could not send path");
    }

    //
    // Render assets
    //

    pub fn convert<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
        params: &G::Params,
    ) -> &G {
        // create new if not in cache
        if !self.render_cache.contains_key(&handle.clone().to_handle()) {
            let asset = self.get(handle.clone()).expect("invalid handle"); // TODO: handle
            let converted = G::convert(asset, params);
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
            let asset = self.get_mut(handle.clone()).expect("invalid handle"); // TODO: hanlde
            let converted = G::convert(asset, params);
            self.render_cache
                .insert(handle.clone().to_handle(), Box::new(converted));
        }

        // get value and convert to G
        self.render_cache
            .get_mut(&handle.to_handle())
            .and_then(|a| a.as_any_mut().downcast_mut::<G>())
            .unwrap()
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
