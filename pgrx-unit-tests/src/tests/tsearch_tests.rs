//LICENSE Portions Copyright 2019-2021 ZomboDB, LLC.
//LICENSE
//LICENSE Portions Copyright 2021-2023 Technology Concepts & Design, Inc.
//LICENSE
//LICENSE Portions Copyright 2023-2023 PgCentral Foundation, Inc. <contact@pgcentral.org>
//LICENSE
//LICENSE All rights reserved.
//LICENSE
//LICENSE Use of this source code is governed by the MIT license that can be found in the LICENSE file.
#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    #[allow(unused_imports)]
    use crate as pgrx_unit_tests;

    use pgrx::prelude::*;
    use pgrx::{TsQuery, TsVector};

    #[pg_extern]
    fn take_and_return_tsvector(v: TsVector) -> TsVector {
        v
    }

    #[pg_extern]
    fn take_and_return_tsquery(q: TsQuery) -> TsQuery {
        q
    }

    #[pg_test]
    fn test_take_and_return_tsvector() {
        let rc = Spi::get_one::<bool>(
            "SELECT tests.take_and_return_tsvector('a:1 b:2'::tsvector) = 'a:1 b:2'::tsvector;",
        );
        assert_eq!(rc, Ok(Some(true)));
    }

    #[pg_test]
    fn test_take_and_return_tsquery() {
        let rc = Spi::get_one::<bool>(
            "SELECT tests.take_and_return_tsquery('a & !b'::tsquery) = 'a & !b'::tsquery;",
        );
        assert_eq!(rc, Ok(Some(true)));
    }

    #[pg_test]
    fn test_tsvector_spi_outlives_call() {
        // value is copied to Rust memory, so it survives the SPI connection that produced it
        let v = Spi::get_one::<TsVector>("SELECT 'fat cat rat'::tsvector").unwrap().unwrap();
        assert_eq!(v.to_string(), "'cat' 'fat' 'rat'");
    }

    #[pg_test]
    fn test_tsquery_display_from_str() {
        let q: TsQuery = "fat & (cat | rat)".parse().unwrap();
        assert_eq!(q.to_string(), "'fat' & ( 'cat' | 'rat' )");
        assert!("fat &".parse::<TsQuery>().is_err());
        assert!("a\0b".parse::<TsVector>().is_err());
    }

    #[pg_test]
    fn test_tsvector_serde() {
        let v: TsVector = serde_json::from_str("\"'a':1 'b':2\"").unwrap();
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"'a':1 'b':2\"");
        assert!(serde_json::from_str::<TsVector>("\"a:x\"").is_err());
    }

    #[pg_test]
    fn test_tsvector_into_datum_roundtrip() {
        let v: TsVector = "'x':3 'y':1".parse().unwrap();
        let back = unsafe { TsVector::from_datum(v.clone().into_datum().unwrap(), false) }.unwrap();
        assert_eq!(back, v);
    }

    #[pg_test]
    fn test_debug_eq_ord() {
        let a: TsVector = "a b".parse().unwrap();
        let b: TsVector = "'b' 'a'".parse().unwrap();
        let c: TsVector = "c".parse().unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        // tsvector_cmp orders by size before lexemes, so just check we agree with SQL
        let sql_lt =
            Spi::get_one::<bool>("SELECT 'a b'::tsvector < 'c'::tsvector").unwrap().unwrap();
        assert_eq!(a < c, sql_lt);
        assert!("a".parse::<TsVector>().unwrap() < "c".parse().unwrap());
        assert_eq!(format!("{a:?}"), "TsVector('a' 'b')");
        let q: TsQuery = "a & b".parse().unwrap();
        assert_eq!(q, "a & b".parse::<TsQuery>().unwrap());
        assert_eq!(format!("{q:?}"), "TsQuery('a' & 'b')");
    }

    #[pg_test]
    fn test_len_default_matches() {
        let v: TsVector = "'fat':2 'cat':3 'rat':1".parse().unwrap();
        assert_eq!(v.len(), 3);
        assert!(!v.is_empty());
        assert!(TsVector::default().is_empty());
        assert_eq!(TsVector::default().to_string(), "");
        assert!(v.matches(&"fat & cat".parse().unwrap()));
        assert!(!v.matches(&"fat & dog".parse().unwrap()));
        assert!(v.matches(&"fat <-> cat".parse().unwrap()));
    }

    #[pg_test]
    fn test_ops() {
        let a: TsVector = "'a':1".parse().unwrap();
        let b: TsVector = "'b':1".parse().unwrap();
        assert_eq!(&a | &b, "'a':1 'b':2".parse().unwrap());
        assert_eq!(a | b, "'a':1 'b':2".parse().unwrap());

        let p: TsQuery = "p".parse().unwrap();
        let q: TsQuery = "q".parse().unwrap();
        assert_eq!(&p & &q, "p & q".parse().unwrap());
        assert_eq!(&p | &q, "p | q".parse().unwrap());
        assert_eq!(!&p, "!p".parse().unwrap());
        assert_eq!(p.followed_by(&q), "p <-> q".parse().unwrap());
        assert_eq!((p.clone() & q.clone()).to_string(), "'p' & 'q'");
        assert_eq!((!p | q).to_string(), "!'p' | 'q'");
    }
}
