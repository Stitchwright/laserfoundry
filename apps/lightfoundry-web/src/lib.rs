//! LightFoundry in the browser: the same engine and egui interface, compiled to WebAssembly.
//!
//! Files open through the browser's file chooser and save as downloads. There is no machine link
//! yet (the browser's Web Serial API is the way to add one). Build it with `apps/lightfoundry-web/build.sh`.
//!
//! The wasm-bindgen glue is generated code, so the workspace's `unsafe_code` lint is relaxed here.
#![allow(unsafe_code)]

pub mod fontindex;
pub mod service;

#[cfg(target_arch = "wasm32")]
mod web {
    use eframe::wasm_bindgen::JsCast;
    use lightfoundry_engine::Session;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use lightfoundry_device::Transport;
    use lightfoundry_ui_egui::{LightFoundryApp, Platform, Upload};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    /// Starts the app on the `<canvas>` with the given id.
    #[wasm_bindgen]
    pub async fn start(canvas_id: String) -> Result<(), JsValue> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .get_element_by_id(&canvas_id)
            .ok_or_else(|| JsValue::from_str("no canvas with that id"))?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|_| JsValue::from_str("that element is not a canvas"))?;
        restore_fonts();
        let leave_warning = Rc::new(Cell::new(false));
        install_leave_warning(&leave_warning);
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| {
                    Ok(Box::new(Host {
                        app: LightFoundryApp::new(
                            Session::new(),
                            Box::new(WebPlatform {
                                inbox: Rc::new(RefCell::new(None)),
                                serial: Rc::new(RefCell::new(None)),
                                font_job: Rc::new(RefCell::new(FontJob::default())),
                                leave_warning,
                                http_job: Rc::new(RefCell::new(HttpJob::default())),
                            }),
                        ),
                    }))
                }),
            )
            .await
    }

    /// Asks the browser to confirm before the page is closed or reloaded while `flag` is set. A page cannot send
    /// anything to the machine while it is being torn down, so a job stopped this way would run on unsupervised.
    fn install_leave_warning(flag: &Rc<Cell<bool>>) {
        let flag = Rc::clone(flag);
        let handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if flag.get() {
                event.prevent_default();
            }
        });
        if let Some(window) = web_sys::window()
            && window.add_event_listener_with_callback("beforeunload", handler.as_ref().unchecked_ref()).is_err()
        {
            log::warn!("could not watch for the page closing");
        }
        handler.forget(); // lives as long as the page
    }

    /// Files go through the browser's file chooser and downloads; there are no paths, serial ports
    /// or threads for a machine link.
    struct WebPlatform {
        /// The chosen file once it has been read (shared with the browser callbacks).
        inbox: Rc<RefCell<Option<Upload>>>,
        /// The serial device once the person has chosen it and it is open (or why not).
        serial: Rc<RefCell<Option<Result<Box<dyn Transport + Send>, String>>>>,
        font_job: Rc<RefCell<FontJob>>,
        /// Whether closing the page would lose work or stop a job halfway (read by the `beforeunload` handler).
        leave_warning: Rc<Cell<bool>>,
        /// The AI or account request in flight (or its answer, not yet taken).
        http_job: Rc<RefCell<HttpJob>>,
    }

    impl Platform for WebPlatform {
        fn pick_open(&self, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<std::path::PathBuf> {
            None
        }

        fn browser_files(&self) -> bool {
            true
        }

        fn request_upload(&self, extensions: &[&str]) {
            if let Err(e) = choose_file(extensions, Rc::clone(&self.inbox)) {
                log::warn!("could not open the file chooser: {e:?}");
            }
        }

        fn take_upload(&self) -> Option<Upload> {
            self.inbox.borrow_mut().take()
        }

        fn download(&self, name: &str, bytes: &[u8], mime: &str) {
            if let Err(e) = start_download(name, bytes, mime) {
                log::warn!("could not start the download: {e:?}");
            }
        }

        fn storage_get(&self, key: &str) -> Option<String> {
            // Storage can be missing or throw (private windows, blocked site data): then nothing is remembered.
            web_sys::window()?.local_storage().ok()??.get_item(key).ok()?
        }

        fn http_available(&self) -> bool {
            BUILT_IN_SERVICE.is_some_and(|s| crate::service::origin(s).is_some())
        }

        fn start_http(&self, request: lightfoundry_ai::HttpRequest) -> bool {
            {
                let mut job = self.http_job.borrow_mut();
                if job.running {
                    return false;
                }
                job.running = true;
                job.result = None;
            }
            let job = Rc::clone(&self.http_job);
            spawn_local(async move {
                let result = if crate::service::allowed(&request.url, BUILT_IN_SERVICE) {
                    post(&request).await
                } else {
                    Err("this page can only talk to the service it was built for".to_string())
                };
                let mut job = job.borrow_mut();
                job.running = false;
                job.result = Some(result);
            });
            true
        }

        fn take_http(&self) -> Option<Result<(u16, String), String>> {
            self.http_job.borrow_mut().result.take()
        }

        fn set_leave_warning(&self, on: bool) {
            self.leave_warning.set(on);
        }

        fn storage_set(&self, key: &str, value: &str) {
            let stored = web_sys::window().and_then(|w| w.local_storage().ok()).flatten().map(|s| s.set_item(key, value));
            if !matches!(stored, Some(Ok(()))) {
                log::warn!("could not remember the recent projects (browser storage is unavailable or full)");
            }
        }

        fn font_library_available(&self) -> bool {
            true
        }

        fn font_library_everything_available(&self) -> bool {
            false
        }

        fn library_fonts(&self) -> Vec<(String, String)> {
            lightfoundry_engine::fontlib::list().into_iter().map(|(name, display)| (display, name)).collect()
        }

        fn start_font_library_download(&self, _everything: bool) -> bool {
            {
                let mut job = self.font_job.borrow_mut();
                if job.running {
                    return false;
                }
                job.running = true;
                job.result = None;
            }
            let job = Rc::clone(&self.font_job);
            spawn_local(async move {
                let result = load_fonts(None).await.map(|folders| {
                    if let (Some(storage), Ok(text)) = (local_storage(), serde_json::to_string(&folders)) {
                        let _ = storage.set_item(FONTS_KEY, &text);
                    }
                    format!("Font library: {} fonts ready under Library fonts in the text inspector.", folders.len())
                });
                let mut job = job.borrow_mut();
                job.running = false;
                job.result = Some(result);
            });
            true
        }

        fn take_font_library_result(&self) -> Option<Result<String, String>> {
            self.font_job.borrow_mut().result.take()
        }

        fn inline_link(&self) -> bool {
            true
        }

        fn serial_async(&self) -> bool {
            true
        }

        fn request_serial(&self) {
            let slot = Rc::clone(&self.serial);
            spawn_local(async move {
                let result = open_serial_port().await.map_err(describe);
                *slot.borrow_mut() = Some(result);
            });
        }

        fn take_serial(&self) -> Option<Result<Box<dyn Transport + Send>, String>> {
            self.serial.borrow_mut().take()
        }

        fn serial_ports(&self) -> Vec<String> {
            if web_serial_available() { vec!["USB serial device".into()] } else { Vec::new() }
        }

        fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
            Err("choose the device in the browser's prompt".into())
        }
    }

    /// Opens the browser's file chooser; the chosen file lands in `inbox` once read.
    fn choose_file(extensions: &[&str], inbox: Rc<RefCell<Option<Upload>>>) -> Result<(), JsValue> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or_else(|| JsValue::from_str("no document"))?;
        let input = document.create_element("input")?.dyn_into::<web_sys::HtmlInputElement>()?;
        input.set_type("file");
        input.set_accept(&extensions.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(","));
        let chosen = input.clone();
        let on_change = Closure::once(move || {
            let Some(file) = chosen.files().and_then(|f| f.get(0)) else { return };
            let name = file.name();
            let Ok(reader) = web_sys::FileReader::new() else { return };
            let done = reader.clone();
            let on_load = Closure::once(move || {
                if let Ok(result) = done.result() {
                    let bytes = js_sys::Uint8Array::new(&result).to_vec();
                    *inbox.borrow_mut() = Some(Upload { name, bytes });
                }
            });
            reader.set_onload(Some(on_load.as_ref().unchecked_ref()));
            on_load.forget();
            if let Err(e) = reader.read_as_array_buffer(&file) {
                log::warn!("could not read the file: {e:?}");
            }
        });
        input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
        on_change.forget();
        input.click();
        Ok(())
    }

    /// Saves `bytes` through a temporary link, which is how a page triggers a download.
    fn start_download(name: &str, bytes: &[u8], mime: &str) -> Result<(), JsValue> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or_else(|| JsValue::from_str("no document"))?;
        let array = js_sys::Uint8Array::from(bytes);
        let parts = js_sys::Array::of1(&array);
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(mime);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let link = document.create_element("a")?.dyn_into::<web_sys::HtmlAnchorElement>()?;
        link.set_href(&url);
        link.set_download(name);
        link.click();
        web_sys::Url::revoke_object_url(&url)
    }

    // ---- Web Serial ----------------------------------------------------------------------------
    // The API is not in web-sys's stable set, so the few calls needed are declared here.

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = ["navigator", "serial"], js_name = requestPort, catch)]
        fn serial_request_port() -> Result<js_sys::Promise, JsValue>;

        type SerialPort;
        #[wasm_bindgen(method, js_name = open)]
        fn open(this: &SerialPort, options: &JsValue) -> js_sys::Promise;
        #[wasm_bindgen(method, getter)]
        fn readable(this: &SerialPort) -> Stream;
        #[wasm_bindgen(method, getter)]
        fn writable(this: &SerialPort) -> Stream;

        type Stream;
        #[wasm_bindgen(method, js_name = getReader)]
        fn get_reader(this: &Stream) -> Reader;
        #[wasm_bindgen(method, js_name = getWriter)]
        fn get_writer(this: &Stream) -> Writer;

        type Reader;
        #[wasm_bindgen(method)]
        fn read(this: &Reader) -> js_sys::Promise;

        type Writer;
        #[wasm_bindgen(method)]
        fn write(this: &Writer, chunk: &JsValue) -> js_sys::Promise;
    }

    /// GRBL's usual speed.
    const BAUD: f64 = 115_200.0;

    fn web_serial_available() -> bool {
        web_sys::window().is_some_and(|w| js_sys::Reflect::has(&w.navigator(), &JsValue::from_str("serial")).unwrap_or(false))
    }

    fn describe(e: JsValue) -> String {
        e.as_string()
            .or_else(|| js_sys::Reflect::get(&e, &JsValue::from_str("message")).ok().and_then(|m| m.as_string()))
            .unwrap_or_else(|| "the browser refused".into())
    }

    /// Asks the person to choose a serial device, opens it and starts the reader.
    async fn open_serial_port() -> Result<Box<dyn Transport + Send>, JsValue> {
        if !web_serial_available() {
            return Err(JsValue::from_str("this browser has no Web Serial (use a Chromium-based browser)"));
        }
        let port: SerialPort = JsFuture::from(serial_request_port()?).await?.unchecked_into();
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &JsValue::from_str("baudRate"), &JsValue::from_f64(BAUD))?;
        JsFuture::from(port.open(&options)).await?;
        let inbox = Rc::new(RefCell::new(Inbox::default()));
        let reader = port.readable().get_reader();
        let reading = Rc::clone(&inbox);
        spawn_local(async move {
            loop {
                let Ok(chunk) = JsFuture::from(reader.read()).await else { break };
                let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done")).ok().and_then(|d| d.as_bool()).unwrap_or(true);
                if done {
                    break;
                }
                if let Ok(value) = js_sys::Reflect::get(&chunk, &JsValue::from_str("value")) {
                    let mut inbox = reading.borrow_mut();
                    inbox.bytes.extend(js_sys::Uint8Array::new(&value).to_vec());
                    // A device at the wrong speed (or a hostile one) may never send a line end: keep only the newest bytes.
                    let excess = inbox.bytes.len().saturating_sub(MAX_INBOX_BYTES);
                    inbox.bytes.drain(..excess);
                }
            }
            reading.borrow_mut().closed = true;
        });
        let outbox = Rc::new(RefCell::new(Outbox::default()));
        Ok(Box::new(WebSerial { inbox, outbox, writer: Rc::new(port.writable().get_writer()) }))
    }

    /// Most unread serial bytes kept (GRBL's replies are a few dozen bytes a line).
    const MAX_INBOX_BYTES: usize = 64 * 1024;

    #[derive(Default)]
    struct Inbox {
        bytes: std::collections::VecDeque<u8>,
        closed: bool,
    }

    #[derive(Default)]
    struct Outbox {
        chunks: std::collections::VecDeque<Vec<u8>>,
        pumping: bool,
    }

    /// A serial device as a non-blocking [`Transport`]: reads come from what the reader task has
    /// collected, and writes are queued for a task that sends them in order.
    struct WebSerial {
        inbox: Rc<RefCell<Inbox>>,
        outbox: Rc<RefCell<Outbox>>,
        writer: Rc<Writer>,
    }

    // SAFETY-ish note: the browser build is single-threaded (wasm32 without atomics has no other
    // thread to move this to), so the `Send` the link's API asks for cannot be violated.
    unsafe impl Send for WebSerial {}

    impl Transport for WebSerial {
        fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            if self.inbox.borrow().closed {
                return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the serial device was closed"));
            }
            let start = {
                let mut out = self.outbox.borrow_mut();
                out.chunks.push_back(bytes.to_vec());
                !std::mem::replace(&mut out.pumping, true)
            };
            if start {
                let (outbox, writer, inbox) = (Rc::clone(&self.outbox), Rc::clone(&self.writer), Rc::clone(&self.inbox));
                spawn_local(async move {
                    loop {
                        let next = {
                            let mut out = outbox.borrow_mut();
                            let next = out.chunks.pop_front();
                            if next.is_none() {
                                out.pumping = false;
                            }
                            next
                        };
                        let Some(chunk) = next else { break };
                        if JsFuture::from(writer.write(&js_sys::Uint8Array::from(chunk.as_slice()))).await.is_err() {
                            inbox.borrow_mut().closed = true;
                            outbox.borrow_mut().pumping = false;
                            break;
                        }
                    }
                });
            }
            Ok(())
        }

        fn read_line(&mut self, _timeout: std::time::Duration) -> std::io::Result<Option<String>> {
            let mut inbox = self.inbox.borrow_mut();
            if let Some(end) = inbox.bytes.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = inbox.bytes.drain(..=end).collect();
                return Ok(Some(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string()));
            }
            if inbox.closed {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the serial device was closed"));
            }
            Ok(None)
        }
    }

    /// Where the installed font folders are remembered between visits.
    const FONTS_KEY: &str = "lightfoundry.fonts";

    /// The font library download in progress (or its result, not yet taken).
    #[derive(Default)]
    struct FontJob {
        running: bool,
        result: Option<Result<String, String>>,
    }

    /// The AI or account request in flight (or its answer, not yet taken).
    #[derive(Default)]
    struct HttpJob {
        running: bool,
        result: Option<Result<(u16, String), String>>,
    }

    /// The service this page was built for (`LF_CLOUD_URL` when the site was built), if any.
    const BUILT_IN_SERVICE: Option<&str> = option_env!("LF_CLOUD_URL");
    /// How long a request may take, and the most of an answer read.
    const HTTP_TIMEOUT_MS: i32 = 240_000;
    const MAX_ANSWER: usize = 16 * 1024 * 1024;

    /// Sends `request` with `fetch` (no redirects, a time limit) and returns the status and body.
    async fn post(request: &lightfoundry_ai::HttpRequest) -> Result<(u16, String), String> {
        let window = web_sys::window().ok_or("no window")?;
        let headers = web_sys::Headers::new().map_err(describe)?;
        for (name, value) in &request.headers {
            headers.set(name, value).map_err(describe)?;
        }
        let controller = web_sys::AbortController::new().map_err(describe)?;
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_headers(&headers);
        init.set_body(&JsValue::from_str(&request.body));
        init.set_redirect(web_sys::RequestRedirect::Error);
        init.set_signal(Some(&controller.signal()));
        let abort = Closure::once_into_js(move || controller.abort());
        let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(abort.unchecked_ref(), HTTP_TIMEOUT_MS).ok();
        let answer = async {
            let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(&request.url, &init))
                .await
                .map_err(describe)?
                .dyn_into()
                .map_err(|_| "the browser gave no response".to_string())?;
            let status = response.status();
            let text = JsFuture::from(response.text().map_err(describe)?).await.map_err(describe)?.as_string().ok_or("the answer is not text")?;
            if text.len() > MAX_ANSWER {
                return Err("the answer was larger than expected".to_string());
            }
            Ok((status, text))
        }
        .await;
        if let Some(timer) = timer {
            window.clear_timeout_with_handle(timer);
        }
        answer
    }

    fn local_storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    /// Fetches a text file from this site (same origin: the fonts are copied onto the site when it is
    /// built, so nothing here talks to a third party). `cap` bounds what is accepted.
    async fn fetch_text(url: &str, cap: usize) -> Result<String, String> {
        let window = web_sys::window().ok_or("no window")?;
        let response: web_sys::Response =
            JsFuture::from(window.fetch_with_str(url)).await.map_err(describe)?.dyn_into().map_err(|_| "the browser gave no response".to_string())?;
        if !response.ok() {
            return Err(format!("{} for {url}", response.status()));
        }
        let text = JsFuture::from(response.text().map_err(describe)?).await.map_err(describe)?.as_string().ok_or("the file is not text")?;
        if text.len() > cap {
            return Err(format!("{url} is larger than expected"));
        }
        Ok(text)
    }

    /// Downloads the fonts the site carries (only `only`, when given) into the engine's font registry.
    /// Returns the folders that loaded.
    async fn load_fonts(only: Option<Vec<String>>) -> Result<Vec<String>, String> {
        let index = fetch_text("fonts/index.json", crate::fontindex::MAX_INDEX_BYTES)
            .await
            .map_err(|e| format!("The font library is not available from this site ({e})"))?;
        let mut loaded = Vec::new();
        for entry in crate::fontindex::parse_index(&index)? {
            if only.as_ref().is_some_and(|names| !names.contains(&entry.folder)) {
                continue;
            }
            let base = format!("fonts/{}/", entry.folder);
            let info = fetch_text(&format!("{base}font.json"), entry.font_json_bytes as usize + 1024).await?;
            let glyphs = fetch_text(&format!("{base}glyphs.stitchfoundry"), entry.glyphs_bytes as usize + 1024).await?;
            match lightfoundry_engine::fontlib::register(&entry.folder, &info, &glyphs) {
                Ok(()) => loaded.push(entry.folder),
                Err(e) => log::warn!("skipping the font {}: {e}", entry.folder),
            }
        }
        if loaded.is_empty() && only.is_none() {
            return Err("The font library on this site has no usable fonts".into());
        }
        Ok(loaded)
    }

    /// Re-fetches the fonts installed on an earlier visit (the browser's cache makes this cheap).
    fn restore_fonts() {
        let remembered =
            local_storage().and_then(|s| s.get_item(FONTS_KEY).ok().flatten()).map(|t| crate::fontindex::parse_remembered(&t)).unwrap_or_default();
        if remembered.is_empty() {
            return;
        }
        spawn_local(async move {
            if let Err(e) = load_fonts(Some(remembered)).await {
                log::warn!("could not restore the installed fonts: {e}");
            }
        });
    }

    struct Host {
        app: LightFoundryApp,
    }

    impl eframe::App for Host {
        fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
            self.app.ui(ui);
        }
    }
}
