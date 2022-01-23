pub(crate) use ::{
    serde::{
        de::{self, Deserialize, Deserializer, Visitor},
        ser::{Serialize, Serializer},
    },
    std::{
        fmt::{self, Formatter},
        fs::{File, OpenOptions},
        io::{self, prelude::*, SeekFrom},
        marker::PhantomData,
        mem,
        ops::{Deref, DerefMut},
        path::{Path, PathBuf},
        slice,
    },
};

fn bytes<T: ?Sized>(x: &T) -> &[u8] {
    unsafe { slice::from_raw_parts(x as *const _ as *const u8, mem::size_of_val(x)) }
}

struct BytesSer<T>(pub T);

impl<T: fmt::Debug> fmt::Debug for BytesSer<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> Serialize for BytesSer<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes(self))
    }
}

struct BytesVisitor<T>(PhantomData<T>);

impl<'a, T: 'a> Visitor<'a> for BytesVisitor<T> {
    type Value = BytesSer<T>;

    fn expecting(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "expecting a byte buffer")
    }

    fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Self::Value, E> {
        Ok(unsafe { bytes.as_ptr().cast::<Self::Value>().read() })
    }
}

impl<'a, T: 'a> Deserialize<'a> for BytesSer<T> {
    fn deserialize<D: Deserializer<'a>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_bytes(BytesVisitor(PhantomData))
    }
}

/// Little wrapper over an [`OpenOptions`],a [`File`] and it's path with the purporse of
/// implementing [`Serialize`] and [`Deserialize`],this wrapper implements exactly the same traits
/// as the [`File`] in the way it does and also [derefs][`Deref`] to it.
#[derive(Debug)]
// todo: remove the unsafe BytesSer wrapper once OpenOptions it's supported in serde
pub struct SerdeFile(BytesSer<OpenOptions>, File, PathBuf);

impl SerdeFile {
    /// Creates a new `Self` opening a [`File`] with [`OpenOptions::open`] on `x` and
    /// the path `path` [`canonicalize`]d.
    pub fn open<P: AsRef<Path>>(x: &OpenOptions, path: P) -> io::Result<Self> {
        // technique copied from the std to being able to inline a function that have generics
        // just make a function that does not have it and inline it
        #[inline]
        fn a(x: &OpenOptions, path: &Path) -> io::Result<SerdeFile> {
            x.open(path).and_then(|file| {
                Ok(SerdeFile(BytesSer(x.clone()), file, path.canonicalize()?))
            })
        }

        a(x, path.as_ref())
    }

    /// Returns a reference to the canonicalized path to the inner `File`.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.2
    }
    
    /// Returns a reference to the `OpenOptions` used to open the inner `File`.
    #[inline]
    pub fn options(&self) -> &OpenOptions {
        &self.0.0
    }

    pub fn into_inner(self) -> (OpenOptions, File, PathBuf) {
        let Self(options, file, path_buf) = self;

        (options.0, file, path_buf)
    }
}

impl Write for SerdeFile {
    #[inline(always)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.1.write(buf)
    }

    #[inline(always)]
    fn flush(&mut self) -> io::Result<()> {
        self.1.flush()
    }
}

impl<'a> Write for &'a SerdeFile {
    #[inline(always)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.1).write(buf)
    }

    #[inline(always)]
    fn flush(&mut self) -> io::Result<()> {
        (&self.1).flush()
    }
}

impl Read for SerdeFile {
    #[inline(always)]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.1.read(buf)
    }
}

impl<'a> Read for &'a SerdeFile {
    #[inline(always)]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.1).read(buf)
    }
}

impl Seek for SerdeFile {
    #[inline(always)]
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.1.seek(pos)
    }
}

impl<'a> Seek for &'a SerdeFile {
    #[inline(always)]
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        (&self.1).seek(pos)
    }
}

impl Deref for SerdeFile {
    type Target = File;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl DerefMut for SerdeFile {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.1
    }
}

impl Serialize for SerdeFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.0, &self.2).serialize(serializer)
    }
}

impl<'a> Deserialize<'a> for SerdeFile
where
    Self: 'a,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        let (options, path_buf) = <(BytesSer<OpenOptions>, PathBuf)>::deserialize(deserializer)?;

        SerdeFile::open(&options.0, &path_buf).map_err(|e| {
            de::Error::custom(format_args!(
                "error with opening {}: {}",
                path_buf.display(),
                e
            ))
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bincode::{deserialize, serialize};

    const S1: &str = "ksad";
    const FILE_PATH: &str = r".\a.txt";

    #[test]
    fn a() {
        scopeguard::defer! {
            let y = FILE_PATH;
            let x = unsafe { (&y as *const &str).read_volatile() };

            std::fs::remove_file(x).unwrap_or_default()
        }

        let mut f = SerdeFile::open(
            OpenOptions::new().read(true).write(true).create(true),
            FILE_PATH,
        )
        .unwrap();

        write!(f, "{}", S1).unwrap();

        let fbytes = serialize(&f).unwrap();

        drop(f);
        let mut f2: SerdeFile = deserialize(&fbytes).unwrap();

        let mut vec = Vec::new();

        f2.read_to_end(&mut vec).unwrap();

        assert_eq!(S1.as_bytes(), &vec);
    }
}
