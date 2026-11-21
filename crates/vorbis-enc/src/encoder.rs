//! The public encoder: planar `f32` PCM in, an Ogg Vorbis stream out to a `Write` sink, in the
//! shape `vorbis_rs` users know (a builder, `encode_audio_block`, `finish`). Pages are cut as
//! libvorbis' example encoder cuts them: the three headers flushed to their own pages, then a
//! page whenever libogg's page-out rule fills one, and a final flush.

use std::fmt;
use std::io::{self, Write};
use std::num::{NonZeroU8, NonZeroU32};

use crate::analysis::PacketEncoder;
use crate::info::pack_comment;
use crate::ogg::OggStream;
use crate::setup::{CodecSetup, SetupError};

/// How the encoder spends bits.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum BitrateManagementStrategy {
    /// Variable bitrate at a quality from −0.2 (smallest) to 1.0 (best); 0.4 is ~128 kbit/s
    /// stereo at 44.1 kHz (`vorbis_encode_init_vbr`).
    QualityVbr {
        /// −0.2 … 1.0 (above 1.0 counts as 1.0; not finite: [`Error::InvalidQuality`])
        target_quality: f32,
    },
    /// Managed bitrate around an average, in bit/s (`vorbis_encode_init(-1, average, -1)`).
    Abr {
        /// bit/s over the stream
        average_bitrate: NonZeroU32,
    },
    /// Managed bitrate under a hard maximum, in bit/s (`vorbis_encode_init(maximum, -1, -1)`).
    ConstrainedAbr {
        /// bit/s, never exceeded over the bitrate window
        maximum_bitrate: NonZeroU32,
    },
    /// Managed bitrate with any of a minimum, an average and a maximum, in bit/s
    /// (`vorbis_encode_init(maximum, average, minimum)`).
    Managed {
        /// bit/s floor
        minimum_bitrate: Option<NonZeroU32>,
        /// bit/s target
        average_bitrate: Option<NonZeroU32>,
        /// bit/s ceiling
        maximum_bitrate: Option<NonZeroU32>,
    },
}

impl Default for BitrateManagementStrategy {
    /// Quality 0.5 (aoTuV's and `vorbis_rs`'s default is VBR; oggenc's default quality is 3,
    /// i.e. 0.3).
    fn default() -> Self {
        BitrateManagementStrategy::QualityVbr {
            target_quality: 0.5,
        }
    }
}

/// Why the encoder could not be built or write.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The VBR quality is NaN or infinite.
    InvalidQuality,
    /// A comment tag is empty or not ASCII 0x20–0x7D without `=` (Vorbis I §5.2.3).
    InvalidCommentTag,
    /// A comment (`TAG=value`) is 4 GiB or longer (its length field is 32 bits).
    CommentTooLong,
    /// [`BitrateManagementStrategy::Managed`] with no bitrate set.
    MissingBitrate,
    /// No aoTuV setup covers this channel count, rate and quality or bitrate (qualities
    /// below −0.2 included).
    Unsupported,
    /// A block has a different number of channels than the encoder (nothing is encoded).
    ChannelMismatch {
        /// the encoder's channel count
        expected: usize,
        /// the block's
        actual: usize,
    },
    /// A block's channels have different lengths (nothing is encoded).
    LengthMismatch,
    /// The stream broke earlier ([`Error::Io`] was returned, or a panic in the encoder or the
    /// sink was caught): it is incomplete and can't be continued.
    StreamBroken,
    /// The sink failed. The stream is incomplete: every later call returns
    /// [`Error::StreamBroken`] (a sink error mid-stream has lost pages).
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidQuality => f.write_str("the VBR quality is not finite"),
            Error::InvalidCommentTag => {
                f.write_str("invalid comment tag (empty, or not ASCII 0x20-0x7D without '=')")
            }
            Error::CommentTooLong => f.write_str("a comment is 4 GiB or longer"),
            Error::MissingBitrate => f.write_str("managed bitrate with no bitrate set"),
            Error::Unsupported => {
                f.write_str("no Vorbis setup for this channel count, rate and quality or bitrate")
            }
            Error::ChannelMismatch { expected, actual } => {
                write!(f, "block has {actual} channels, the encoder {expected}")
            }
            Error::LengthMismatch => f.write_str("block channels differ in length"),
            Error::StreamBroken => {
                f.write_str("the stream is incomplete after an earlier sink failure")
            }
            Error::Io(_) => f.write_str("writing to the sink failed"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<SetupError> for Error {
    fn from(e: SetupError) -> Self {
        match e {
            // the public API's rates and channel counts are in range: only no bitrate is left
            SetupError::Invalid => Error::MissingBitrate,
            SetupError::Unsupported => Error::Unsupported,
        }
    }
}

/// Configures an [`Encoder`].
pub struct EncoderBuilder<W: Write> {
    rate: NonZeroU32,
    channels: NonZeroU8,
    sink: W,
    strategy: BitrateManagementStrategy,
    serial: Option<i32>,
    comments: Vec<Vec<u8>>,
    /// the first invalid comment's error, reported by `build`
    bad_comment: Option<Error>,
}

impl<W: Write> fmt::Debug for EncoderBuilder<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncoderBuilder")
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .field("strategy", &self.strategy)
            .field("serial", &self.serial)
            .field("comments", &self.comments.len())
            .finish_non_exhaustive()
    }
}

impl<W: Write> EncoderBuilder<W> {
    /// An encoder of `channels` channels at `rate` Hz into `sink` (default: VBR quality 0.5,
    /// a random stream serial number, no comments).
    ///
    /// Channels are in Vorbis order (Vorbis I §4.3.9): 1 mono; 2 L R; 3 L C R; 4 FL FR RL RR;
    /// 5 FL C FR RL RR; 6 FL C FR RL RR LFE (5.1: aoTuV codes the last channel as the LFE,
    /// low-passed); 7 FL C FR SL SR RC LFE; 8 FL C FR SL SR RL RR LFE; above 8 the
    /// application's. WAV/SMPTE order (L R C LFE …) must be reordered first.
    pub fn new(rate: NonZeroU32, channels: NonZeroU8, sink: W) -> Self {
        EncoderBuilder {
            rate,
            channels,
            sink,
            strategy: BitrateManagementStrategy::default(),
            serial: None,
            comments: Vec::new(),
            bad_comment: None,
        }
    }

    /// How bits are spent.
    #[must_use]
    pub fn bitrate_management_strategy(mut self, strategy: BitrateManagementStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// The Ogg stream's serial number (unique within a chained or multiplexed file). The
    /// default is random per process where std has a random source (on
    /// `wasm32-unknown-unknown` it is the same every run: set one).
    #[must_use]
    pub fn stream_serial(mut self, serial: i32) -> Self {
        self.serial = Some(serial);
        self
    }

    /// A `TAG=value` user comment (e.g. `TITLE`, `ARTIST`); added in call order. The tag must
    /// be non-empty ASCII 0x20–0x7D without `=` (else [`build`](Self::build) returns
    /// [`Error::InvalidCommentTag`]); the value is any UTF-8, written verbatim (a NUL included,
    /// which libvorbis' C-string API cuts off; the whole comment under 4 GiB, else
    /// [`Error::CommentTooLong`]).
    #[must_use]
    pub fn comment_tag(mut self, tag: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let tag = tag.as_ref().as_bytes();
        let bad_tag = tag.is_empty()
            || tag
                .iter()
                .any(|&c| !(0x20..=0x7d).contains(&c) || c == b'=');
        let mut c = tag.to_vec();
        c.push(b'=');
        c.extend_from_slice(value.as_ref().as_bytes());
        if self.bad_comment.is_none() {
            if bad_tag {
                self.bad_comment = Some(Error::InvalidCommentTag);
            } else if u32::try_from(c.len()).is_err() {
                self.bad_comment = Some(Error::CommentTooLong);
            }
        }
        self.comments.push(c);
        self
    }

    /// Sets the encoder up and writes the three header packets' pages to the sink.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidCommentTag`] or [`Error::CommentTooLong`] for the first invalid
    /// comment; [`Error::InvalidQuality`] for a quality that is not finite;
    /// [`Error::MissingBitrate`] for `Managed` with no bitrate; [`Error::Unsupported`] when no
    /// aoTuV setup covers the channel count, rate and quality or bitrate; [`Error::Io`] when
    /// the sink fails.
    pub fn build(self) -> Result<Encoder<W>, Error> {
        if let Some(e) = self.bad_comment {
            return Err(e);
        }
        let ch = self.channels.get() as i64;
        let rate = self.rate.get() as i64;
        let b = |v: Option<NonZeroU32>| v.map_or(-1, |v| v.get() as i64);
        let ci = match self.strategy {
            BitrateManagementStrategy::QualityVbr { target_quality } => {
                // the C takes NaN and +inf as the top quality: rejected here
                if !target_quality.is_finite() {
                    return Err(Error::InvalidQuality);
                }
                CodecSetup::vbr(ch, rate, target_quality)?
            }
            BitrateManagementStrategy::Abr { average_bitrate } => {
                CodecSetup::managed(ch, rate, -1, average_bitrate.get() as i64, -1)?
            }
            BitrateManagementStrategy::ConstrainedAbr { maximum_bitrate } => {
                CodecSetup::managed(ch, rate, maximum_bitrate.get() as i64, -1, -1)?
            }
            BitrateManagementStrategy::Managed {
                minimum_bitrate,
                average_bitrate,
                maximum_bitrate,
            } => CodecSetup::managed(
                ch,
                rate,
                b(maximum_bitrate),
                b(average_bitrate),
                b(minimum_bitrate),
            )?,
        };
        let serial = self.serial.unwrap_or_else(random_serial);
        let enc = PacketEncoder::new(ci);
        let [h0, _, h2] = enc.headers();
        let comments: Vec<&[u8]> = self.comments.iter().map(|c| &c[..]).collect();
        let h1 = pack_comment(&comments);
        let mut os = OggStream::new(serial);
        os.packetin(&h0, false, 0);
        os.packetin(&h1, false, 0);
        os.packetin(&h2, false, 0);
        let mut e = Encoder {
            enc,
            os,
            sink: Some(self.sink),
            channels: self.channels.get() as usize,
            failed: true, // until the headers are out: a sink error here drops `e` unwritten
        };
        while let Some(page) = e.os.flush() {
            e.sink()?.write_all(&page)?;
        }
        e.failed = false;
        Ok(e)
    }
}

/// A random serial number without a dependency: std's per-process random hash keys.
fn random_serial() -> i32 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(0u8) as i32
}

/// An Ogg Vorbis encoder writing to `W`. Dropping it finishes the stream (errors ignored);
/// call [`Encoder::finish`] to see them and get the sink back.
///
/// After an [`Error::Io`] the stream is incomplete and every later call returns
/// [`Error::StreamBroken`];
/// dropping it then writes nothing more. A sink error is final: `ErrorKind::Interrupted` and
/// short writes are retried (`write_all`), but `WouldBlock` breaks the stream, so a
/// non-blocking sink needs a buffer in between.
pub struct Encoder<W: Write> {
    enc: PacketEncoder,
    os: OggStream,
    sink: Option<W>,
    channels: usize,
    /// a sink error (pages lost) or a panic mid-encode: the stream can't be continued
    failed: bool,
}

impl<W: Write> fmt::Debug for Encoder<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encoder")
            .field("channels", &self.channels)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl<W: Write> Encoder<W> {
    fn sink(&mut self) -> io::Result<&mut W> {
        self.sink
            .as_mut()
            .ok_or_else(|| io::Error::other("encoder finished"))
    }

    /// The error every call returns once the stream is broken.
    fn check(&self) -> Result<(), Error> {
        if self.failed {
            return Err(Error::StreamBroken);
        }
        Ok(())
    }

    /// Encodes one block of planar samples (one slice per channel in Vorbis order, see
    /// [`EncoderBuilder::new`]; all one length); writes whatever pages complete. Any block
    /// length works, empty included.
    ///
    /// Samples are nominally −1.0 … 1.0, but any `f32` is accepted (NaN and ±inf included)
    /// without a panic. The output is aoTuV's byte for byte given the same sequence of block
    /// lengths (the first blocks set the start-of-stream extrapolation), for samples up to
    /// ±2^20; beyond that aoTuV's own output depends on the CPU (C undefined behaviour).
    ///
    /// # Errors
    ///
    /// [`Error::ChannelMismatch`] when the block's channel count differs from the encoder's,
    /// [`Error::LengthMismatch`] when its channels' lengths differ (nothing is encoded; the
    /// encoder stays usable); [`Error::Io`] when the sink fails now, [`Error::StreamBroken`]
    /// when the stream broke before.
    pub fn encode_audio_block<B: AsRef<[f32]>>(
        &mut self,
        block: impl IntoIterator<Item = B>,
    ) -> Result<(), Error> {
        self.check()?;
        let block: Vec<B> = block.into_iter().collect();
        if block.len() != self.channels {
            return Err(Error::ChannelMismatch {
                expected: self.channels,
                actual: block.len(),
            });
        }
        let pcm: Vec<&[f32]> = block.iter().map(|c| c.as_ref()).collect();
        if pcm.iter().any(|c| c.len() != pcm[0].len()) {
            return Err(Error::LengthMismatch);
        }
        if pcm[0].is_empty() {
            return Ok(()); // an empty write would end the stream in the C API
        }
        // until the pages are out: a panic in the encoder or the sink leaves the stream broken
        self.failed = true;
        let packets = self.enc.write(&pcm);
        self.emit(packets, false)
    }

    /// Pages the packets out to the sink; a sink error breaks the stream for good.
    fn emit(&mut self, packets: Vec<crate::analysis::Packet>, flush: bool) -> Result<(), Error> {
        let r = self.emit_inner(packets, flush);
        self.failed = r.is_err();
        r
    }

    fn emit_inner(
        &mut self,
        packets: Vec<crate::analysis::Packet>,
        flush: bool,
    ) -> Result<(), Error> {
        for p in packets {
            self.os.packetin(&p.data, p.eos, p.granulepos);
            while let Some(page) = self.os.pageout() {
                self.sink()?.write_all(&page)?;
            }
        }
        if flush {
            while let Some(page) = self.os.flush() {
                self.sink()?.write_all(&page)?;
            }
        }
        Ok(())
    }

    /// Ends the stream once: afterwards `failed` or the sink taken keeps `Drop` from ending it
    /// again.
    fn end(&mut self) -> Result<(), Error> {
        self.check()?;
        self.failed = true;
        let packets = self.enc.finish();
        self.emit(packets, true)?;
        let r = self.sink()?.flush().map_err(Error::from);
        self.failed = r.is_err();
        r
    }

    /// Ends the stream (the last pages, end-of-stream flag set) and returns the sink.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the sink fails now, [`Error::StreamBroken`] when the stream broke
    /// before; the sink is dropped either way.
    pub fn finish(mut self) -> Result<W, Error> {
        let r = self.end();
        let sink = self.sink.take().expect("the sink until finish");
        r.map(|()| sink)
    }
}

impl<W: Write> Drop for Encoder<W> {
    fn drop(&mut self) {
        // not after a failure, nor while unwinding: a panic mid-encode leaves the state
        // inconsistent, and a second panic here would abort the process
        if self.sink.is_some() && !self.failed && !std::thread::panicking() {
            let _ = self.end();
        }
    }
}
