use std::ffi::{c_int, c_void};
use std::ptr::{self, NonNull};

use cut::Timebase;
use ffmpeg_sys_next as sys;

use super::{AGAIN, CodecParams, Frame, Packet, StreamParams, check, codec_name, timebase};
use crate::MediaError;

/// A hardware decoding API FFmpeg can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwDevice {
    D3d11va,
    Dxva2,
    Vaapi,
    VideoToolbox,
}

impl HwDevice {
    /// The APIs worth trying on this platform, best first.
    pub fn preferred() -> &'static [HwDevice] {
        if cfg!(windows) {
            &[HwDevice::D3d11va, HwDevice::Dxva2]
        } else if cfg!(target_os = "macos") {
            &[HwDevice::VideoToolbox]
        } else {
            &[HwDevice::Vaapi]
        }
    }

    fn raw(self) -> sys::AVHWDeviceType {
        match self {
            HwDevice::D3d11va => sys::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
            HwDevice::Dxva2 => sys::AVHWDeviceType::AV_HWDEVICE_TYPE_DXVA2,
            HwDevice::Vaapi => sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            HwDevice::VideoToolbox => sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
        }
    }
}

/// What a receive call produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Received {
    /// The output argument now holds a frame or packet.
    Output,
    /// More input is needed first.
    Again,
    /// Fully drained.
    Eof,
}

/// An opened decoder or encoder.
pub struct CodecContext {
    ptr: NonNull<sys::AVCodecContext>,
    hardware: bool,
}

// SAFETY: a codec context is only used through `&mut self` and has no thread affinity; FFmpeg's
// internal worker threads are managed by the context itself.
unsafe impl Send for CodecContext {}

impl CodecContext {
    fn allocate(codec: *const sys::AVCodec) -> Result<CodecContext, MediaError> {
        // SAFETY: `codec` is a valid codec or null; null allocates a generic context.
        let raw = unsafe { sys::avcodec_alloc_context3(codec) };
        NonNull::new(raw)
            .map(|ptr| CodecContext {
                ptr,
                hardware: false,
            })
            .ok_or(MediaError::OutOfMemory)
    }

    fn raw(&self) -> &sys::AVCodecContext {
        // SAFETY: the context is valid for the lifetime of `self`.
        unsafe { self.ptr.as_ref() }
    }

    /// Opens a decoder for `stream`, using `hardware` when the codec and system support it.
    /// Hardware setup failures fall back to software silently.
    pub fn decoder(
        stream: &StreamParams,
        hardware: Option<HwDevice>,
    ) -> Result<CodecContext, MediaError> {
        let params = stream.view();
        let codec_id = params.raw().codec_id;
        // SAFETY: avcodec_find_decoder returns a static codec or null.
        let codec = unsafe { sys::avcodec_find_decoder(codec_id) };
        if codec.is_null() {
            return Err(MediaError::NoDecoder(codec_name(codec_id)));
        }
        let mut context = CodecContext::allocate(codec)?;
        let raw = context.ptr.as_ptr();
        // SAFETY: both pointers are valid; this copies the stream's parameters into the context.
        let code = unsafe { sys::avcodec_parameters_to_context(raw, params.raw()) };
        check(code, "avcodec_parameters_to_context")?;
        // SAFETY: we exclusively own the unopened context, so writing its fields is sound.
        unsafe {
            (*raw).pkt_timebase = stream.raw_timebase();
            (*raw).thread_count = 0;
        }
        if let Some(device) = hardware {
            context.hardware = context.attach_hardware(codec, device);
        }
        // SAFETY: the context was allocated for `codec` and is opened once.
        let code = unsafe { sys::avcodec_open2(raw, codec, ptr::null_mut()) };
        check(code, "avcodec_open2")?;
        Ok(context)
    }

    fn attach_hardware(&mut self, codec: *const sys::AVCodec, device: HwDevice) -> bool {
        let Some(pixel_format) = hardware_pixel_format(codec, device) else {
            return false;
        };
        let mut device_context = ptr::null_mut();
        // SAFETY: `device_context` is a valid out-pointer; null device name and options select
        // the default adapter.
        let code = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut device_context,
                device.raw(),
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        if code < 0 || device_context.is_null() {
            return false;
        }
        let raw = self.ptr.as_ptr();
        // SAFETY: the context is unopened and exclusively ours; it takes ownership of the device
        // reference and frees it on close. `opaque` smuggles the wanted pixel format to the
        // get_format callback as an integer, never dereferenced.
        unsafe {
            (*raw).hw_device_ctx = device_context;
            (*raw).opaque = pixel_format as isize as *mut c_void;
            (*raw).get_format = Some(pick_hardware_format);
        }
        true
    }

    /// Whether the decoder was set up for hardware. Individual frames may still come back as
    /// software frames if FFmpeg declined the hardware format.
    pub fn is_hardware(&self) -> bool {
        self.hardware
    }

    pub fn timebase(&self) -> Option<Timebase> {
        timebase(self.raw().time_base)
    }

    /// Feeds a packet, or `None` to start draining. Returns `false` if the codec wants its
    /// output read first; the packet must then be sent again.
    pub fn send_packet(&mut self, packet: Option<&Packet>) -> Result<bool, MediaError> {
        let packet = packet.map_or(ptr::null(), Packet::as_ptr);
        // SAFETY: the context is open; the packet is valid or null for drain.
        let code = unsafe { sys::avcodec_send_packet(self.ptr.as_ptr(), packet) };
        match code {
            AGAIN => Ok(false),
            sys::AVERROR_EOF => Ok(true),
            _ => check(code, "avcodec_send_packet").map(|_| true),
        }
    }

    pub fn receive_frame(&mut self, frame: &mut Frame) -> Result<Received, MediaError> {
        // SAFETY: the context is open and the frame valid; FFmpeg unrefs the frame first.
        let code = unsafe { sys::avcodec_receive_frame(self.ptr.as_ptr(), frame.as_mut_ptr()) };
        received(code, "avcodec_receive_frame")
    }

    /// Feeds a frame, or `None` to drain. Returns `false` if packets must be read first.
    pub fn send_frame(&mut self, frame: Option<&Frame>) -> Result<bool, MediaError> {
        let frame = frame.map_or(ptr::null(), Frame::as_ptr);
        // SAFETY: the context is an open encoder; the frame is valid or null for drain.
        let code = unsafe { sys::avcodec_send_frame(self.ptr.as_ptr(), frame) };
        match code {
            AGAIN => Ok(false),
            sys::AVERROR_EOF => Ok(true),
            _ => check(code, "avcodec_send_frame").map(|_| true),
        }
    }

    pub fn receive_packet(&mut self, packet: &mut Packet) -> Result<Received, MediaError> {
        // SAFETY: the context is an open encoder and the packet valid.
        let code = unsafe { sys::avcodec_receive_packet(self.ptr.as_ptr(), packet.as_mut_ptr()) };
        received(code, "avcodec_receive_packet")
    }

    /// Drops buffered frames, e.g. after a seek.
    pub fn flush(&mut self) {
        // SAFETY: the context is open.
        unsafe { sys::avcodec_flush_buffers(self.ptr.as_ptr()) };
    }

    pub(super) fn as_mut_ptr(&mut self) -> *mut sys::AVCodecContext {
        self.ptr.as_ptr()
    }

    pub(super) fn as_ptr(&self) -> *const sys::AVCodecContext {
        self.ptr.as_ptr()
    }

    /// Opens an audio encoder producing a stream like `source`: `codec` when given, otherwise
    /// the source's own codec. Tries the sample formats and rates encoders commonly accept.
    pub fn audio_encoder(
        source: &CodecParams<'_>,
        codec: Option<&str>,
        bit_rate: u64,
        global_header: bool,
    ) -> Result<CodecContext, MediaError> {
        let encoder = match codec {
            Some(name) => {
                let name_c = super::c_string(name)?;
                // SAFETY: the name is NUL-terminated; returns a static codec or null.
                unsafe { sys::avcodec_find_encoder_by_name(name_c.as_ptr()) }
            }
            // SAFETY: returns a static codec or null.
            None => unsafe { sys::avcodec_find_encoder(source.raw().codec_id) },
        };
        if encoder.is_null() {
            let wanted = codec.map_or_else(|| source.codec_name(), str::to_owned);
            return Err(MediaError::NoEncoder(wanted));
        }
        use sys::AVSampleFormat as F;
        let formats = [
            F::AV_SAMPLE_FMT_FLTP,
            F::AV_SAMPLE_FMT_FLT,
            F::AV_SAMPLE_FMT_S16P,
            F::AV_SAMPLE_FMT_S16,
            F::AV_SAMPLE_FMT_S32P,
            F::AV_SAMPLE_FMT_S32,
        ];
        let rates = [source.raw().sample_rate, 48_000];
        let mut last_error = MediaError::NoEncoder(codec_name(source.raw().codec_id));
        for rate in rates.into_iter().filter(|&r| r > 0) {
            for format in formats {
                match Self::try_open_audio_encoder(
                    encoder,
                    source,
                    format,
                    rate,
                    bit_rate,
                    global_header,
                ) {
                    Ok(context) => return Ok(context),
                    Err(error) => last_error = error,
                }
            }
        }
        Err(last_error)
    }

    fn try_open_audio_encoder(
        encoder: *const sys::AVCodec,
        source: &CodecParams<'_>,
        format: sys::AVSampleFormat,
        sample_rate: c_int,
        bit_rate: u64,
        global_header: bool,
    ) -> Result<CodecContext, MediaError> {
        let context = CodecContext::allocate(encoder)?;
        let raw = context.ptr.as_ptr();
        // SAFETY: the context is unopened and exclusively ours; the source layout is initialized.
        unsafe {
            check(
                sys::av_channel_layout_copy(&mut (*raw).ch_layout, &source.raw().ch_layout),
                "av_channel_layout_copy",
            )?;
            if (*raw).ch_layout.nb_channels <= 0 {
                sys::av_channel_layout_default(&mut (*raw).ch_layout, 2);
            }
            (*raw).sample_fmt = format;
            (*raw).sample_rate = sample_rate;
            (*raw).time_base = sys::AVRational {
                num: 1,
                den: sample_rate,
            };
            (*raw).bit_rate = i64::try_from(bit_rate).unwrap_or(i64::MAX);
            if global_header {
                (*raw).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
            }
            check(
                sys::avcodec_open2(raw, encoder, ptr::null_mut()),
                "avcodec_open2",
            )?;
        }
        Ok(context)
    }

    /// Opens the video encoder named `name` for frames like `first`, copying color and aspect
    /// signalling from `source`. Writes codec headers in-band (no global header) so encoded
    /// packets can be spliced into a stream that carries different ones. No B-frames, and one
    /// keyframe at the start.
    pub fn video_encoder(
        name: &str,
        source: &CodecParams<'_>,
        first: &Frame,
        timebase: Timebase,
        frame_rate: Option<(i32, i32)>,
        options: &[(&str, &str)],
    ) -> Result<CodecContext, MediaError> {
        let name_c = super::c_string(name)?;
        // SAFETY: the name is NUL-terminated; returns a static codec or null.
        let encoder = unsafe { sys::avcodec_find_encoder_by_name(name_c.as_ptr()) };
        if encoder.is_null() {
            return Err(MediaError::NoEncoder(name.to_owned()));
        }
        let format = super::pixel_format(first.format())
            .ok_or_else(|| MediaError::Unsupported("frame pixel format".into()))?;
        let context = CodecContext::allocate(encoder)?;
        let raw = context.ptr.as_ptr();
        let params = source.raw();
        let (aspect_num, aspect_den) = first.sample_aspect_ratio();
        let mut dictionary = ptr::null_mut();
        // SAFETY: the context is unopened and exclusively ours; option strings are
        // NUL-terminated; the dictionary is freed after opening whatever happens.
        unsafe {
            (*raw).width = c_int::try_from(first.width()).unwrap_or(0);
            (*raw).height = c_int::try_from(first.height()).unwrap_or(0);
            (*raw).pix_fmt = format;
            (*raw).time_base = super::rational(timebase);
            if let Some((num, den)) = frame_rate {
                (*raw).framerate = sys::AVRational { num, den };
            }
            (*raw).sample_aspect_ratio = sys::AVRational {
                num: aspect_num.max(0),
                den: aspect_den.max(1),
            };
            // The decoded frames' range wins: conversions on the way may have changed it.
            (*raw).color_range = match first.raw_color_range() {
                sys::AVColorRange::AVCOL_RANGE_UNSPECIFIED => params.color_range,
                range => range,
            };
            (*raw).colorspace = params.color_space;
            (*raw).color_primaries = params.color_primaries;
            (*raw).color_trc = params.color_trc;
            (*raw).chroma_sample_location = params.chroma_location;
            (*raw).max_b_frames = 0;
            (*raw).gop_size = 100_000;
            (*raw).thread_count = 0;
            for (key, value) in options {
                let (key, value) = (super::c_string(key)?, super::c_string(value)?);
                sys::av_dict_set(&mut dictionary, key.as_ptr(), value.as_ptr(), 0);
            }
            let code = sys::avcodec_open2(raw, encoder, &mut dictionary);
            sys::av_dict_free(&mut dictionary);
            check(code, "avcodec_open2")?;
        }
        Ok(context)
    }

    /// Whether FFmpeg has an encoder by this name.
    pub fn encoder_exists(name: &str) -> bool {
        let Ok(name) = super::c_string(name) else {
            return false;
        };
        // SAFETY: the name is NUL-terminated; returns a static codec or null.
        !unsafe { sys::avcodec_find_encoder_by_name(name.as_ptr()) }.is_null()
    }

    /// Samples per frame the encoder requires, or `None` if any size is accepted.
    pub fn frame_size(&self) -> Option<u32> {
        // SAFETY: the codec pointer of an opened context is a valid static codec.
        let variable = unsafe { self.raw().codec.as_ref() }.is_some_and(|codec| {
            codec.capabilities & sys::AV_CODEC_CAP_VARIABLE_FRAME_SIZE as c_int != 0
        });
        let size = u32::try_from(self.raw().frame_size).unwrap_or(0);
        (!variable && size > 0).then_some(size)
    }

    pub fn sample_rate(&self) -> u32 {
        u32::try_from(self.raw().sample_rate).unwrap_or(0)
    }

    pub fn sample_format_name(&self) -> Option<String> {
        super::sample_format_name(self.raw().sample_fmt as c_int)
    }

    pub fn channel_layout_description(&self) -> String {
        // SAFETY: an opened audio codec's layout is initialized.
        unsafe { super::describe_layout(&self.raw().ch_layout) }
    }

    pub fn codec_name(&self) -> String {
        codec_name(self.raw().codec_id)
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        let mut raw = self.ptr.as_ptr();
        // SAFETY: allocated by avcodec_alloc_context3 and freed exactly once; this also closes
        // the codec and releases the hardware device reference.
        unsafe { sys::avcodec_free_context(&mut raw) };
    }
}

fn received(code: c_int, operation: &'static str) -> Result<Received, MediaError> {
    match code {
        AGAIN => Ok(Received::Again),
        sys::AVERROR_EOF => Ok(Received::Eof),
        _ => check(code, operation).map(|_| Received::Output),
    }
}

fn hardware_pixel_format(codec: *const sys::AVCodec, device: HwDevice) -> Option<c_int> {
    (0..)
        .map_while(|index| {
            // SAFETY: `codec` is a valid static codec; configs end with null.
            unsafe { sys::avcodec_get_hw_config(codec, index).as_ref() }
        })
        .find(|config| {
            config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as c_int != 0
                && config.device_type == device.raw()
        })
        .map(|config| config.pix_fmt as c_int)
}

/// Picks the hardware format stored in `opaque` if offered, otherwise the first software format,
/// which is how decoding falls back to software when the hardware can't take the stream.
unsafe extern "C" fn pick_hardware_format(
    context: *mut sys::AVCodecContext,
    formats: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    // SAFETY: FFmpeg passes the context we configured and a list terminated by AV_PIX_FMT_NONE.
    unsafe {
        let wanted = (*context).opaque as isize as c_int;
        let mut software = sys::AVPixelFormat::AV_PIX_FMT_NONE;
        let mut cursor = formats;
        while *cursor != sys::AVPixelFormat::AV_PIX_FMT_NONE {
            if *cursor as c_int == wanted {
                return *cursor;
            }
            let is_hardware = sys::av_pix_fmt_desc_get(*cursor)
                .as_ref()
                .is_some_and(|d| d.flags & sys::AV_PIX_FMT_FLAG_HWACCEL as u64 != 0);
            if !is_hardware && software == sys::AVPixelFormat::AV_PIX_FMT_NONE {
                software = *cursor;
            }
            cursor = cursor.add(1);
        }
        software
    }
}
