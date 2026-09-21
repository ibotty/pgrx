//LICENSE Portions Copyright 2019-2021 ZomboDB, LLC.
//LICENSE
//LICENSE Portions Copyright 2021-2023 Technology Concepts & Design, Inc.
//LICENSE
//LICENSE Portions Copyright 2023-2023 PgCentral Foundation, Inc. <contact@pgcentral.org>
//LICENSE
//LICENSE All rights reserved.
//LICENSE
//LICENSE Use of this source code is governed by the MIT license that can be found in the LICENSE file.
//! `tsvector` and `tsquery` full-text search types.
use crate::{FromDatum, IntoDatum, direct_function_call, direct_function_call_as_datum, pg_sys};
use alloc::ffi::CString;
use core::ffi::CStr;
use pgrx_pg_sys::PgTryBuilder;
use pgrx_pg_sys::panic::CaughtError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::ops::{BitAnd, BitOr, Not};
use std::str::FromStr;

macro_rules! tsearch_type {
    ($name:ident, $sql:literal, $oid:ident, $infn:ident, $outfn:ident, $cmpfn:ident) => {
        #[doc = concat!("A `", $sql, "` type from PostgreSQL, held as Rust-owned varlena bytes")]
        #[derive(Clone)]
        pub struct $name(Box<[u8]>);

        impl $name {
            /// A borrowed Datum view over our bytes, valid only while `self` lives.
            #[inline]
            fn as_datum(&self) -> Option<pg_sys::Datum> {
                Some(pg_sys::Datum::from(self.0.as_ptr()))
            }

            /// Call a Postgres function returning a palloc'd `$sql`, copy it into Rust memory, free it.
            unsafe fn from_call(
                f: unsafe fn(pg_sys::FunctionCallInfo) -> pg_sys::Datum,
                args: &[Option<pg_sys::Datum>],
            ) -> Self {
                unsafe {
                    let datum = direct_function_call_as_datum(f, args).expect("returned NULL");
                    let value = Self::from_datum(datum, false).unwrap();
                    pg_sys::pfree(datum.cast_mut_ptr());
                    value
                }
            }
        }

        impl FromDatum for $name {
            unsafe fn from_polymorphic_datum(
                datum: pg_sys::Datum,
                is_null: bool,
                _typoid: pg_sys::Oid,
            ) -> Option<Self> {
                if is_null {
                    return None;
                }
                // same dance as AnyNumeric: detoast, copy into Rust memory, free the detoast copy
                unsafe {
                    let varlena = pg_sys::pg_detoast_datum(datum.cast_mut_ptr());
                    let is_copy = !std::ptr::eq(varlena, datum.cast_mut_ptr::<pg_sys::varlena>());
                    let size = crate::varsize_any(varlena);
                    let boxed: Box<[u8]> =
                        std::slice::from_raw_parts(varlena.cast::<u8>(), size).into();
                    if is_copy {
                        pg_sys::pfree(varlena.cast());
                    }
                    Some($name(boxed))
                }
            }
        }

        impl IntoDatum for $name {
            fn into_datum(self) -> Option<pg_sys::Datum> {
                unsafe {
                    let dest = pg_sys::palloc(self.0.len()).cast::<u8>();
                    std::ptr::copy_nonoverlapping(self.0.as_ptr(), dest, self.0.len());
                    Some(pg_sys::Datum::from(dest))
                }
            }

            fn type_oid() -> pg_sys::Oid {
                pg_sys::$oid
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                unsafe {
                    let cstr = direct_function_call::<&CStr>(pg_sys::$outfn, &[self.as_datum()])
                        .expect(concat!(stringify!($outfn), " returned NULL"));
                    let result = f.write_str(cstr.to_str().map_err(|_| fmt::Error)?);
                    pg_sys::pfree(cstr.as_ptr().cast_mut().cast());
                    result
                }
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({self})", stringify!($name))
            }
        }

        impl FromStr for $name {
            type Err = Box<CaughtError>;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                // bad input raises a Postgres ERROR (or a panic for an embedded NUL); both land in `Err`
                PgTryBuilder::new(|| unsafe {
                    let cstr = CString::new(s).expect("embedded NUL byte");
                    Ok(Self::from_call(pg_sys::$infn, &[cstr.as_c_str().into_datum()]))
                })
                .catch_others(|e| Err(Box::new(e)))
                .execute()
            }
        }

        impl Ord for $name {
            fn cmp(&self, other: &Self) -> Ordering {
                let cmp: i32 = unsafe {
                    direct_function_call(pg_sys::$cmpfn, &[self.as_datum(), other.as_datum()])
                        .unwrap()
                };
                cmp.cmp(&0)
            }
        }

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.cmp(other) == Ordering::Equal
            }
        }

        impl Eq for $name {}

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                s.parse().map_err(|e| serde::de::Error::custom(format!("invalid {}: {e:?}", $sql)))
            }
        }

        crate::impl_sql_translatable!($name, $sql);
    };
}

tsearch_type!(TsVector, "tsvector", TSVECTOROID, tsvectorin, tsvectorout, tsvector_cmp);
tsearch_type!(TsQuery, "tsquery", TSQUERYOID, tsqueryin, tsqueryout, tsquery_cmp);

/// A binary Postgres operator on owned or borrowed operands.
macro_rules! tsearch_binop {
    ($ty:ident, $trait:ident, $method:ident, $pgfn:ident) => {
        impl $trait<&$ty> for &$ty {
            type Output = $ty;
            fn $method(self, rhs: &$ty) -> $ty {
                unsafe { $ty::from_call(pg_sys::$pgfn, &[self.as_datum(), rhs.as_datum()]) }
            }
        }
        impl $trait for $ty {
            type Output = $ty;
            fn $method(self, rhs: $ty) -> $ty {
                (&self).$method(&rhs)
            }
        }
    };
}

// `||`: concatenate two vectors
tsearch_binop!(TsVector, BitOr, bitor, tsvector_concat);
// `&&`, `||`: combine two queries
tsearch_binop!(TsQuery, BitAnd, bitand, tsquery_and);
tsearch_binop!(TsQuery, BitOr, bitor, tsquery_or);

impl Not for &TsQuery {
    type Output = TsQuery;
    /// `!!`: negate the query
    fn not(self) -> TsQuery {
        unsafe { TsQuery::from_call(pg_sys::tsquery_not, &[self.as_datum()]) }
    }
}

impl Not for TsQuery {
    type Output = TsQuery;
    fn not(self) -> TsQuery {
        !&self
    }
}

impl TsQuery {
    /// `<->`: `self` followed immediately by `other`
    pub fn followed_by(&self, other: &TsQuery) -> TsQuery {
        unsafe { TsQuery::from_call(pg_sys::tsquery_phrase, &[self.as_datum(), other.as_datum()]) }
    }
}

impl TsVector {
    /// Number of lexemes, as `length(tsvector)`
    pub fn len(&self) -> usize {
        // Box<[u8]> is only 1-aligned, so read the int32 header field from the bytes
        let at = std::mem::offset_of!(pg_sys::TSVectorData, size);
        i32::from_ne_bytes(self.0[at..at + 4].try_into().unwrap()) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `@@`: does this vector match the query?
    pub fn matches(&self, query: &TsQuery) -> bool {
        unsafe {
            direct_function_call(pg_sys::ts_match_vq, &[self.as_datum(), query.as_datum()]).unwrap()
        }
    }
}

impl Default for TsQuery {
    /// The empty query, `''::tsquery`; matches nothing
    fn default() -> Self {
        // header only (vl_len_ + size = 0), built by hand because tsqueryin('') emits a NOTICE
        let len = std::mem::size_of::<pg_sys::TSQueryData>();
        let mut bytes = vec![0u8; len];
        bytes[..4].copy_from_slice(&crate::varlena::encode_vlen_4b(len as i32).to_ne_bytes());
        TsQuery(bytes.into())
    }
}

impl Default for TsVector {
    /// The empty vector, `''::tsvector`
    fn default() -> Self {
        "".parse().unwrap()
    }
}
