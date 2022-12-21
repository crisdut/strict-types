// Strict encoding schema library, implementing validation and parsing
// strict encoded data against a schema.
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2022-2023 by
//     Dr. Maxim Orlovsky <orlovsky@ubideco.org>
//
// Copyright 2022-2023 Ubideco Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::io::{BufRead, Seek};
use std::{fs, io};

use amplify::confinement::{Collection, Confined};
use amplify::num::u24;

use super::DecodeError;
use crate::encoding::{DeserializeError, SerializeError, StrictReader, StrictWriter};
use crate::Ident;

pub trait ToIdent {
    fn to_ident(&self) -> Ident;
}
impl ToIdent for &'static str {
    fn to_ident(&self) -> Ident { Ident::from(*self) }
}
impl ToIdent for String {
    fn to_ident(&self) -> Ident { Ident::try_from(self.to_owned()).expect("invalid identifier") }
}
impl ToIdent for Ident {
    fn to_ident(&self) -> Ident { self.clone() }
}
pub trait ToMaybeIdent {
    fn to_maybe_ident(&self) -> Option<Ident>;
}
impl<T> ToMaybeIdent for Option<T>
where T: ToIdent
{
    fn to_maybe_ident(&self) -> Option<Ident> { self.as_ref().map(|n| n.to_ident()) }
}

pub trait TypedWrite: Sized {
    type TupleWriter: WriteTuple<Self>;
    type StructWriter: WriteStruct<Self>;
    type UnionDefiner: DefineUnion<Parent = Self>;
    type EnumDefiner: DefineEnum<Parent = Self>;

    // TODO: Remove optionals
    fn define_union(self, ns: impl ToIdent, name: Option<impl ToIdent>) -> Self::UnionDefiner;
    fn define_enum(self, ns: impl ToIdent, name: Option<impl ToIdent>) -> Self::EnumDefiner;

    fn write_tuple(self, ns: impl ToIdent, name: Option<impl ToIdent>) -> Self::TupleWriter;
    fn write_type(
        self,
        ns: impl ToIdent,
        name: Option<impl ToIdent>,
        value: &impl StrictEncode,
    ) -> io::Result<Self> {
        Ok(self.write_tuple(ns, name).write_field(value)?.complete())
    }
    fn write_struct(self, ns: impl ToIdent, name: Option<impl ToIdent>) -> Self::StructWriter;

    #[doc(hidden)]
    unsafe fn _write_raw<const MAX_LEN: usize>(self, bytes: impl AsRef<[u8]>) -> io::Result<Self>;
    #[doc(hidden)]
    unsafe fn write_raw_array<const LEN: usize>(self, raw: [u8; LEN]) -> io::Result<Self> {
        self._write_raw::<LEN>(raw)
    }
    #[doc(hidden)]
    unsafe fn write_raw_bytes<const MAX_LEN: usize>(
        self,
        bytes: impl AsRef<[u8]>,
    ) -> io::Result<Self> {
        self.write_raw_len::<MAX_LEN>(bytes.as_ref().len())?._write_raw::<MAX_LEN>(bytes)
    }
    #[doc(hidden)]
    unsafe fn write_raw_len<const MAX_LEN: usize>(self, len: usize) -> io::Result<Self> {
        match MAX_LEN {
            tiny if tiny <= u8::MAX as usize => u8::strict_encode(&(len as u8), self),
            small if small < u16::MAX as usize => u16::strict_encode(&(len as u16), self),
            medium if medium < u24::MAX.into_usize() => {
                u24::strict_encode(&u24::with(len as u32), self)
            }
            large if large < u32::MAX as usize => u32::strict_encode(&(len as u32), self),
            _ => unreachable!("confined collections larger than u32::MAX must not exist"),
        }
    }
    #[doc(hidden)]
    unsafe fn write_raw_collection<C: Collection, const MIN_LEN: usize, const MAX_LEN: usize>(
        mut self,
        col: &Confined<C, MIN_LEN, MAX_LEN>,
    ) -> io::Result<Self>
    where
        for<'a> &'a C: IntoIterator,
        for<'a> <&'a C as IntoIterator>::Item: StrictEncode,
    {
        self = self.write_raw_len::<MAX_LEN>(col.len())?;
        for item in col {
            self = item.strict_encode(self)?;
        }
        Ok(self)
    }
}

pub trait DefineTuple<P: Sized>: Sized {
    fn define_field<T: StrictEncode>(self) -> Self;
    fn define_field_ord<T: StrictEncode>(self, ord: u8) -> Self;
    fn complete(self) -> P;
}

pub trait WriteTuple<P: Sized>: Sized {
    fn write_field(self, value: &impl StrictEncode) -> io::Result<Self>;
    fn write_field_ord(self, ord: u8, value: &impl StrictEncode) -> io::Result<Self>;
    fn complete(self) -> P;
}

pub trait DefineStruct<P: Sized>: Sized {
    fn define_field<T: StrictEncode>(self, name: impl ToIdent) -> Self;
    fn define_field_ord<T: StrictEncode>(self, name: impl ToIdent, ord: u8) -> Self;
    fn complete(self) -> P;
}

pub trait WriteStruct<P: Sized>: Sized {
    fn write_field(self, name: impl ToIdent, value: &impl StrictEncode) -> io::Result<Self>;
    fn write_field_ord(
        self,
        name: impl ToIdent,
        ord: u8,
        value: &impl StrictEncode,
    ) -> io::Result<Self>;
    fn complete(self) -> P;
}

pub trait DefineEnum: Sized {
    type Parent: TypedWrite;
    type EnumWriter: WriteEnum<Parent = Self::Parent>;
    fn define_variant(self, name: impl ToIdent, value: u8) -> Self;
    fn complete(self) -> Self::EnumWriter;
}

pub trait WriteEnum: Sized {
    type Parent: TypedWrite;
    fn write_variant(self, name: impl ToIdent) -> io::Result<Self>;
    fn complete(self) -> Self::Parent;
}

pub trait DefineUnion: Sized {
    type Parent: TypedWrite;
    type TupleDefiner: DefineTuple<Self>;
    type StructDefiner: DefineStruct<Self>;
    type UnionWriter: WriteUnion<Parent = Self::Parent>;

    fn define_unit(self, name: impl ToIdent) -> Self;
    fn define_type<T: StrictEncode>(self, name: impl ToIdent) -> Self {
        self.define_tuple(name).define_field::<T>().complete()
    }
    fn define_tuple(self, name: impl ToIdent) -> Self::TupleDefiner;
    fn define_struct(self, name: impl ToIdent) -> Self::StructDefiner;

    fn complete(self) -> Self::UnionWriter;
}

pub trait WriteUnion: Sized {
    type Parent: TypedWrite;
    type TupleWriter: WriteTuple<Self>;
    type StructWriter: WriteStruct<Self>;

    fn write_unit(self, name: impl ToIdent) -> io::Result<Self>;
    fn write_type(self, name: impl ToIdent, value: &impl StrictEncode) -> io::Result<Self> {
        Ok(self.write_tuple(name)?.write_field(value)?.complete())
    }
    fn write_tuple(self, name: impl ToIdent) -> io::Result<Self::TupleWriter>;
    fn write_struct(self, name: impl ToIdent) -> io::Result<Self::StructWriter>;

    fn complete(self) -> Self::Parent;
}

pub trait TypedRead: Sized {}

pub trait StrictEncode: Sized {
    type Dumb: StrictEncode = Self;
    fn strict_encode_dumb() -> Self::Dumb;
    fn strict_encode<W: TypedWrite>(&self, writer: W) -> io::Result<W>;
}

pub trait StrictDecode: Sized {
    fn strict_decode(reader: &impl TypedRead) -> Result<Self, DecodeError>;
}

impl<T: StrictEncode<Dumb = T>> StrictEncode for &T {
    type Dumb = T;
    fn strict_encode_dumb() -> T { T::strict_encode_dumb() }
    fn strict_encode<W: TypedWrite>(&self, writer: W) -> io::Result<W> {
        (*self).strict_encode(writer)
    }
}

pub trait Serialize: StrictEncode {
    fn strict_serialized_len(&self) -> io::Result<usize> {
        let counter = StrictWriter::counter();
        Ok(self.strict_encode(counter)?.unbox().count)
    }

    fn to_strict_serialized<const MAX: usize>(
        &self,
    ) -> Result<Confined<Vec<u8>, 0, MAX>, SerializeError> {
        let ast_data = StrictWriter::in_memory(MAX);
        let data = self.strict_encode(ast_data)?.unbox();
        Confined::<Vec<u8>, 0, MAX>::try_from(data).map_err(SerializeError::from)
    }

    fn strict_serialize_to_file<const MAX: usize>(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), SerializeError> {
        let file = StrictWriter::with(MAX, fs::File::create(path)?);
        self.strict_encode(file)?;
        Ok(())
    }
}

pub trait Deserialize: StrictDecode {
    fn from_strict_serialized<const MAX: usize>(
        ast_data: Confined<Vec<u8>, 0, MAX>,
    ) -> Result<Self, DeserializeError> {
        let cursor = io::Cursor::new(ast_data.into_inner());
        let mut reader = StrictReader::with(MAX, cursor);
        let me = Self::strict_decode(&mut reader)?;
        let mut cursor = reader.unbox();
        if !cursor.fill_buf()?.is_empty() {
            return Err(DeserializeError::DataNotEntirelyConsumed);
        }
        Ok(me)
    }

    fn strict_deserialize_from_file<const MAX: usize>(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, DeserializeError> {
        let file = fs::File::open(path)?;
        let mut reader = StrictReader::with(MAX, file);
        let me = Self::strict_decode(&mut reader)?;
        let mut file = reader.unbox();
        if file.stream_position()? != file.seek(io::SeekFrom::End(0))? {
            return Err(DeserializeError::DataNotEntirelyConsumed);
        }
        Ok(me)
    }
}
