use crate::handle::AssetHandle;
use std::any::TypeId;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

pub type DynAsset = Box<dyn Asset>;
pub type DynRenderAsset = ArcHandle<dyn Any + Send + Sync>;
pub type DynAssetLoadFn = Box<dyn Fn(&Path) -> DynAsset>;
pub type DynAssetWriteFn = Box<dyn Fn(&mut DynAsset, &Path)>;

pub trait Asset: Any + Send + Sync {}

pub trait LoadableAsset {
    fn load(path: &Path) -> Self;
}
pub trait WriteableAsset {
    fn write(&mut self, _path: &Path);
}

pub trait RenderAsset: Any {}

pub trait ConvertableRenderAsset: RenderAsset + Send + Sync {
    type SourceAsset: Asset;
    type Params;

    fn convert(source: &Self::SourceAsset, params: &Self::Params) -> Self;
}

pub struct Assets {
    cache: HashMap<AssetHandle<DynAsset>, DynAsset>,
    render_cache: HashMap<AssetHandle<DynAsset>, DynRenderAsset>,

    load_handles: HashMap<AssetHandle<DynAsset>, PathBuf>,
    load_dirty: HashSet<AssetHandle<DynAsset>>,

    // async loading
    load_sender: mpsc::Sender<(AssetHandle<DynAsset>, DynAsset)>,
    load_receiver: mpsc::Receiver<(AssetHandle<DynAsset>, DynAsset)>,

    // reloading
    reload_functions: HashMap<TypeId, DynAssetLoadFn>,
    reload_handles: HashMap<PathBuf, Vec<AssetHandle<DynAsset>>>, // TODO: support multiple assets with same path
    reload_watcher: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::FsEventWatcher>,
    reload_receiver: mpsc::Receiver<PathBuf>,
    reload_sender: mpsc::Sender<PathBuf>,

    // writing
    write_functions: HashMap<TypeId, DynAssetWriteFn>,
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
            load_dirty: HashSet::new(),
            reload_handles: HashMap::new(),
            load_handles: HashMap::new(),

            write_functions: HashMap::new(),

            reload_functions: HashMap::new(),
            reload_receiver,
            reload_sender,
            reload_watcher,

            load_sender: loaded_sender,
            load_receiver: loaded_receiver,
        }
    }

    //
    // Assets
    //

    pub fn insert<T: Asset + 'static>(&mut self, data: T) -> AssetHandle<T> {
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().clone_typed::<DynAsset>(), Box::new(data));
        handle
    }

    // TODO: add get_or_default (e.g. 1x1 white pixel for image)
    //
    // could return error union [Ok, Invalid, Loading]
    pub fn get<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> Option<&T> {
        self.cache
            .get(&handle.clone_typed::<DynAsset>())
            .map(|asset| {
                asset
                    .as_any()
                    .downcast_ref::<T>()
                    .expect("could not downcast")
            })
    }

    pub fn get_mut<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> Option<&mut T> {
        // invalidate gpu cache
        self.render_cache
            .remove(&handle.clone().clone_typed::<DynAsset>());

        // set dirty
        self.load_dirty
            .insert(handle.clone().clone_typed::<DynAsset>());

        // get value and convert to T
        self.cache
            .get_mut(&handle.clone_typed::<DynAsset>())
            .map(|asset| {
                asset
                    .as_any_mut()
                    .downcast_mut::<T>()
                    .expect("could not downcast")
            })
    }

    //
    // Reloading
    //

    pub fn load<T: Asset + LoadableAsset + WriteableAsset>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
        sync: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();
        let handle = AssetHandle::<T>::new();

        if sync {
            let data = T::load(&path);
            self.cache
                .insert(handle.clone().clone_typed::<DynAsset>(), Box::new(data));
        } else {
            let path_clone = path.clone();
            let handle_clone = handle.clone();
            let loaded_sender_clone = self.load_sender.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20000));
                let data = T::load(&path_clone);
                loaded_sender_clone
                    .send((handle_clone.clone_typed::<DynAsset>(), Box::new(data)))
                    .expect("could not send");
            });
        }

        if watch {
            self.watch(handle.clone(), path.clone());
        }

        if write {
            self.write(handle.clone(), path.clone());
        }

        handle
    }

    pub fn load_sync<T: Asset + LoadableAsset + WriteableAsset>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let data = T::load(&path);
        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().clone_typed::<DynAsset>(), Box::new(data));

        if watch {
            self.watch(handle.clone(), path.clone());
        }

        if write {
            self.write(handle.clone(), path.clone());
        }

        handle
    }

    pub fn load_async<T: Asset + LoadableAsset + WriteableAsset>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let handle = AssetHandle::<T>::new();

        let path_clone = path.clone();
        let handle_clone = handle.clone();
        let loaded_sender_clone = self.load_sender.clone();

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5000)); // TODO: remove debug
            let data = T::load(&path_clone);
            loaded_sender_clone
                .send((handle_clone.clone_typed::<DynAsset>(), Box::new(data)))
                .expect("could not send");
        });

        if watch {
            self.watch(handle.clone(), path.clone());
        }

        if write {
            self.write(handle.clone(), path.clone());
        }

        handle
    }

    fn watch<T: Asset + LoadableAsset>(&mut self, handle: AssetHandle<T>, path: PathBuf) {
        // start watching path
        self.reload_watcher
            .watcher()
            .watch(
                &path,
                notify_debouncer_mini::notify::RecursiveMode::Recursive,
            )
            .unwrap();

        // map path to handle
        let handles = self.reload_handles.entry(path).or_default();
        handles.push(handle.clone_typed::<DynAsset>());

        // store reload function
        self.reload_functions
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(|path| Box::new(T::load(path))));
    }
    fn write<T: Asset + WriteableAsset>(&mut self, handle: AssetHandle<T>, path: PathBuf) {
        // map handle to path
        self.load_handles
            .insert(handle.clone_typed::<DynAsset>(), path.clone());

        // store reload function
        self.write_functions
            .entry(TypeId::of::<T>())
            .or_insert_with(|| {
                Box::new(|asset, path| {
                    let typed = asset
                        .as_any_mut()
                        .downcast_mut::<T>()
                        .expect("could not cast during write");
                    typed.write(path);
                })
            });
    }

    //
    // Render assets
    //

    pub fn convert<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
        params: &G::Params,
    ) -> Option<ArcHandle<G>> {
        // create new if not in cache
        if !self
            .render_cache
            .contains_key(&handle.clone().clone_typed::<DynAsset>())
        {
            let asset = self.get(handle.clone());

            if let Some(asset) = asset {
                let converted = G::convert(asset, params);
                self.render_cache.insert(
                    handle.clone().clone_typed::<DynAsset>(),
                    ArcHandle::new(converted).upcast(),
                );
            }
        }

        // get value and convert to G
        self.render_cache
            .get(&handle.clone_typed::<DynAsset>())
            .map(|a| a.downcast::<G>())
    }

    //
    // Polling
    //

    // check if any files completed loading and update cache and invalidate render cache
    pub fn poll_loaded(&mut self) {
        for (handle, asset) in self.load_receiver.try_iter() {
            self.cache.insert(handle.clone(), asset);
            self.render_cache.remove(&handle);
        }
    }

    // check if any files are scheduled for writing to disk
    pub fn poll_write(&mut self) {
        for handle in self.load_dirty.drain() {
            if let Some(path) = self.load_handles.get(&handle) {
                let asset = self.cache.get_mut(&handle);

                // write if loaded
                if let Some(asset) = asset {
                    let write_fn = self
                        .write_functions
                        .get(&handle.ty_id)
                        .expect("could not get write fn");

                    write_fn(asset, path);
                }
            }
        }
    }

    // checks if any files changed and spawns a thread which reloads the data
    pub fn poll_reload(&mut self) {
        for path in self.reload_receiver.try_iter() {
            if let Some(handles) = self.reload_handles.get_mut(&path) {
                for handle in handles {
                    println!("reload {:?}", path);

                    // create/overwrite current value
                    let loader_fn = self
                        .reload_functions
                        .get(&handle.ty_id)
                        .expect("could not get loader fn");
                    let asset = loader_fn(&path);
                    self.cache.insert(handle.clone(), asset);

                    // invalidate render cache
                    self.render_cache.remove(handle);
                }
            }
        }
    }

    pub fn force_reload(&self, path: PathBuf) {
        self.reload_sender.send(path).expect("could not send path");
    }
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

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct ArcHandle<T: ?Sized + 'static> {
    pub handle: Arc<T>,
    id: u64,
}

impl<T: 'static> ArcHandle<T> {
    pub fn new(handle: T) -> Self {
        ArcHandle {
            handle: Arc::new(handle),
            id: NEXT_ID.fetch_add(1, SeqCst),
        }
    }

    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl<T: 'static> Clone for ArcHandle<T> {
    fn clone(&self) -> Self {
        ArcHandle {
            handle: Arc::clone(&self.handle),
            id: self.id,
        }
    }
}

impl<T: 'static> PartialEq for ArcHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for ArcHandle<T> {}

impl<T: 'static> std::hash::Hash for ArcHandle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> std::ops::Deref for ArcHandle<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.handle.as_ref()
    }
}

impl<T: 'static> AsRef<T> for ArcHandle<T> {
    fn as_ref(&self) -> &T {
        self.handle.as_ref()
    }
}

// any stuff

impl<T: Any + Send + Sync + 'static> ArcHandle<T> {
    pub fn upcast(self) -> ArcHandle<dyn Any + Send + Sync> {
        ArcHandle {
            handle: self.handle as Arc<dyn Any + Send + Sync>,
            id: self.id,
        }
    }
}
impl ArcHandle<dyn Any + Sync + Send> {
    fn downcast<G: Send + Sync>(&self) -> ArcHandle<G> {
        ArcHandle {
            handle: self
                .handle
                .clone()
                .downcast::<G>()
                .expect("could not downcast"),
            id: self.id,
        }
    }
}
