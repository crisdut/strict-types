// Strict encoding schema library, implementing validation and parsing
// strict encoded data against a schema.
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2022-2023 by
//     Dr. Maxim Orlovsky <orlovsky@ubideco.org>
//
// Copyright 2022-2023 UBIDECO Institute
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

//! Reification module: reads & writes strict values from binary strict encodings.

use std::io;

use amplify::confinement::{
    LargeAscii, LargeString, MediumAscii, MediumString, SmallAscii, SmallString, TinyAscii,
    TinyString,
};
use amplify::num::u24;
use encoding::constants::*;
use encoding::{DecodeError, StrictDecode, StrictReader};
use indexmap::IndexMap;

use crate::typify::{TypeSpec, TypedVal};
use crate::{SemId, StrictVal, Ty, TypeRef, TypeSystem};

#[derive(Clone, Eq, PartialEq, Debug, Display, Error, From)]
#[display(doc_comments)]
pub enum Error {
    /// unknown type `{0}`.
    TypeAbsent(TypeSpec),

    /// {0} is not yet implemented. Please update `strict_types` to the latest version.
    NotImplemented(String),

    #[display(inner)]
    #[from]
    Decode(DecodeError),
}

impl TypeSystem {
    pub fn store(
        &self,
        spec: impl Into<TypeSpec>,
        obj: TypedVal,
        e: impl io::Write,
    ) -> Result<(), Error> {
        todo!()
    }

    fn read_list(
        &self,
        len: usize,
        ty: SemId,
        d: &mut impl io::Read,
    ) -> Result<Vec<StrictVal>, Error> {
        let mut list = Vec::with_capacity(len);
        for _ in 0..len {
            let item = self.load(ty, d)?;
            list.push(item.val);
        }
        Ok(list)
    }

    fn read_map(
        &self,
        len: usize,
        key_ty: SemId,
        ty: SemId,
        d: &mut impl io::Read,
    ) -> Result<Vec<(StrictVal, StrictVal)>, Error> {
        let mut list = Vec::with_capacity(len);
        for _ in 0..len {
            let key = self.load(key_ty, d)?;
            let item = self.load(ty, d)?;
            list.push((key.val, item.val));
        }
        Ok(list)
    }

    pub fn load(
        &self,
        spec: impl Into<TypeSpec>,
        mut d: &mut impl io::Read,
    ) -> Result<TypedVal, Error> {
        let spec = spec.into();
        let ty = &self.find(&spec).ok_or_else(|| Error::TypeAbsent(spec.clone()))?.ty;

        let mut reader = StrictReader::with(usize::MAX, d);

        let val = match ty {
            Ty::Primitive(prim) => {
                match *prim {
                    U8 => StrictVal::num(u8::strict_decode(&mut reader)?),
                    U16 => StrictVal::num(u16::strict_decode(&mut reader)?),
                    U24 => StrictVal::num(u24::strict_decode(&mut reader)?.into_u32()),
                    U32 => StrictVal::num(u32::strict_decode(&mut reader)?),
                    U64 => StrictVal::num(u64::strict_decode(&mut reader)?),
                    U128 => StrictVal::num(u128::strict_decode(&mut reader)?),
                    I8 => StrictVal::num(i8::strict_decode(&mut reader)?),
                    I16 => StrictVal::num(i16::strict_decode(&mut reader)?),
                    // I24 => StrictVal::num(i24::strict_decode(&mut reader)?),
                    I32 => StrictVal::num(i32::strict_decode(&mut reader)?),
                    I64 => StrictVal::num(i64::strict_decode(&mut reader)?),
                    I128 => StrictVal::num(i128::strict_decode(&mut reader)?),
                    other => {
                        return Err(Error::NotImplemented(format!(
                            "loading {other} into a typed value is not yet implemented"
                        )))
                    }
                }
            }
            Ty::UnicodeChar => {
                todo!()
            }
            Ty::Enum(variants) => {
                let tag = u8::strict_decode(&mut reader)?;
                if !variants.has_tag(tag) {
                    return Err(DecodeError::EnumTagNotKnown(spec.to_string(), tag).into());
                }
                StrictVal::enumer(tag)
            }
            Ty::Union(variants) => {
                let tag = u8::strict_decode(&mut reader)?;
                let Some(ty) = variants.ty_by_ord(tag) else {
                    return Err(DecodeError::EnumTagNotKnown(spec.to_string(), tag).into());
                };
                let fields = self.load(*ty, reader.unbox())?;
                StrictVal::union(tag, fields.val)
            }
            Ty::Tuple(reqs) => {
                let mut fields = Vec::with_capacity(reqs.len());
                let d = reader.unbox();
                for ty in reqs {
                    let checked = self.load(*ty, d)?;
                    fields.push(checked.val);
                }
                StrictVal::tuple(fields)
            }
            Ty::Struct(reqs) => {
                let mut fields = IndexMap::with_capacity(reqs.len());
                let d = reader.unbox();
                for field in reqs {
                    let checked = self.load(field.ty, d)?;
                    fields.insert(field.name.clone(), checked.val);
                }
                StrictVal::Struct(fields)
            }
            Ty::Array(_ty, _len) => {
                todo!()
            }

            // Unicode strings:
            Ty::List(ty, sizing) if ty.is_unicode_char() && sizing.max <= u8::MAX as u64 => {
                let string = TinyString::strict_decode(&mut reader)?;
                StrictVal::String(string.into_inner())
            }
            Ty::List(ty, sizing) if ty.is_unicode_char() && sizing.max <= u16::MAX as u64 => {
                let string = SmallString::strict_decode(&mut reader)?;
                StrictVal::String(string.into_inner())
            }
            Ty::List(ty, sizing) if ty.is_unicode_char() && sizing.max <= u24::MAX.into_u64() => {
                let string = MediumString::strict_decode(&mut reader)?;
                StrictVal::String(string.into_inner())
            }
            Ty::List(ty, sizing) if ty.is_unicode_char() && sizing.max <= u32::MAX as u64 => {
                let string = LargeString::strict_decode(&mut reader)?;
                StrictVal::String(string.into_inner())
            }

            // ASCII strings:
            Ty::List(ty, sizing) if ty.is_ascii_char() && sizing.max <= u8::MAX as u64 => {
                let string = TinyAscii::strict_decode(&mut reader)?;
                StrictVal::String(string.to_string())
            }
            Ty::List(ty, sizing) if ty.is_ascii_char() && sizing.max <= u16::MAX as u64 => {
                let string = SmallAscii::strict_decode(&mut reader)?;
                StrictVal::String(string.to_string())
            }
            Ty::List(ty, sizing) if ty.is_ascii_char() && sizing.max <= u24::MAX.into_u64() => {
                let string = MediumAscii::strict_decode(&mut reader)?;
                StrictVal::String(string.to_string())
            }
            Ty::List(ty, sizing) if ty.is_ascii_char() && sizing.max <= u32::MAX as u64 => {
                let string = LargeAscii::strict_decode(&mut reader)?;
                StrictVal::String(string.to_string())
            }

            // Other lists:
            Ty::List(ty, sizing) if sizing.max <= u8::MAX as u64 => {
                let len = u8::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::List(list)
            }
            Ty::List(ty, sizing) if sizing.max <= u16::MAX as u64 => {
                let len = u16::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::List(list)
            }
            Ty::List(ty, sizing) if sizing.max <= u24::MAX.into_u64() => {
                let len = u24::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len.into_usize(), *ty, d)?;
                StrictVal::List(list)
            }
            Ty::List(ty, sizing) if sizing.max <= u32::MAX as u64 => {
                let len = u32::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::List(list)
            }
            Ty::List(ty, _) => {
                let len = u64::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::List(list)
            }
            // TODO: Find a way to check for the uniqueness of the set values
            Ty::Set(ty, sizing) if sizing.max <= u8::MAX as u64 => {
                let len = u8::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::Set(list)
            }
            Ty::Set(ty, sizing) if sizing.max <= u16::MAX as u64 => {
                let len = u16::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::Set(list)
            }
            Ty::Set(ty, sizing) if sizing.max <= u24::MAX.into_u64() => {
                let len = u24::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len.into_usize(), *ty, d)?;
                StrictVal::Set(list)
            }
            Ty::Set(ty, sizing) if sizing.max <= u32::MAX as u64 => {
                let len = u32::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::Set(list)
            }
            Ty::Set(ty, _) => {
                let len = u64::strict_decode(&mut reader)?;
                d = reader.unbox();
                let list = self.read_list(len as usize, *ty, d)?;
                StrictVal::Set(list)
            }
            Ty::Map(key_ty, ty, sizing) if sizing.max <= u8::MAX as u64 => {
                let len = u8::strict_decode(&mut reader)?;
                let key_ty = key_ty.to_ty().id(None);
                d = reader.unbox();
                let list = self.read_map(len as usize, key_ty, *ty, d)?;
                StrictVal::Map(list)
            }
            Ty::Map(key_ty, ty, sizing) if sizing.max <= u16::MAX as u64 => {
                let len = u16::strict_decode(&mut reader)?;
                let key_ty = key_ty.to_ty().id(None);
                d = reader.unbox();
                let list = self.read_map(len as usize, key_ty, *ty, d)?;
                StrictVal::Map(list)
            }
            Ty::Map(key_ty, ty, sizing) if sizing.max <= u24::MAX.into_u64() => {
                let len = u24::strict_decode(&mut reader)?;
                let key_ty = key_ty.to_ty().id(None);
                d = reader.unbox();
                let list = self.read_map(len.into_usize(), key_ty, *ty, d)?;
                StrictVal::Map(list)
            }
            Ty::Map(key_ty, ty, sizing) if sizing.max <= u32::MAX as u64 => {
                let len = u32::strict_decode(&mut reader)?;
                let key_ty = key_ty.to_ty().id(None);
                d = reader.unbox();
                let list = self.read_map(len as usize, key_ty, *ty, d)?;
                StrictVal::Map(list)
            }
            Ty::Map(key_ty, ty, sizing) => {
                let len = u64::strict_decode(&mut reader)?;
                let key_ty = key_ty.to_ty().id(None);
                d = reader.unbox();
                let list = self.read_map(len as usize, key_ty, *ty, d)?;
                StrictVal::Map(list)
            }
        };

        Ok(TypedVal { val, spec })
    }
}

#[cfg(test)]
mod test {
    use super::super::test_helpers::*;
    // use super::*;

    #[test]
    fn typify() {
        let sys = test_system();
        //let nominal = Nominal::with("TICK", "Some name", 2);
        let value = svstruct!(name => "Some name", ticker => "TICK", precision => svenum!(2));
        let checked = sys.typify(value, "TestLib.Nominal").unwrap();
        assert_eq!(
            format!("{}", checked.val),
            r#"(name="Some name", ticker="TICK", precision=twoDecimals)"#
        );
    }
}
