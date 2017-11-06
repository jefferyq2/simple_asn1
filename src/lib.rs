extern crate num;
#[cfg(test)]
#[macro_use]
extern crate quickcheck;

use num::{BigInt,BigUint,FromPrimitive,One,ToPrimitive,Zero};
use std::iter::FromIterator;
use std::mem::size_of;

#[derive(Clone,Debug,PartialEq)]
pub enum ASN1Block {
    Boolean(ASN1Class, bool),
    Integer(ASN1Class, BigInt),
    BitString(ASN1Class, usize, Vec<u8>),
    OctetString(ASN1Class, Vec<u8>),
    Null(ASN1Class),
    ObjectIdentifier(ASN1Class, Vec<BigUint>),
    IA5String(ASN1Class, String),
    UTF8String(ASN1Class, String),
    Sequence(ASN1Class, Vec<ASN1Block>),
    Set(ASN1Class, Vec<ASN1Block>),
    Unknown(ASN1Class, BigUint, Vec<u8>)
}

#[derive(Clone,Copy,Debug,PartialEq)]
pub enum ASN1Class { Universal, Application, ContextSpecific, Private }

#[derive(Clone,Debug,PartialEq)]
pub enum ASN1DecodeErr {
    EmptyBuffer,
    BadBooleanLength,
    LengthTooLarge,
    UTF8DecodeFailure
}

#[derive(Clone,Debug,PartialEq)]
pub enum ASN1EncodeErr {
    ObjectIdentHasTooFewFields,
    ObjectIdentVal1TooLarge,
    ObjectIdentVal2TooLarge
}

pub fn from_der(i: &[u8]) -> Result<Vec<ASN1Block>,ASN1DecodeErr> {
    let mut result: Vec<ASN1Block> = Vec::new();
    let mut index:  usize          = 0;
    let     len                    = i.len();

    while index < len {
        let (tag, class) = decode_tag(i, &mut index);
        let len = decode_length(i, &mut index)?;
        let body = &i[index .. (index + len)];

        match tag.to_u8() {
            // BOOLEAN
            Some(0x01) => {
                if len != 1 {
                    return Err(ASN1DecodeErr::BadBooleanLength);
                }
                result.push(ASN1Block::Boolean(class, body[0] != 0));
            }
            // INTEGER
            Some(0x02) => {
                let res = BigInt::from_signed_bytes_be(&body);
                result.push(ASN1Block::Integer(class, res));
            }
            // BIT STRING
            Some(0x03) if body.len() == 0 => {
                result.push(ASN1Block::BitString(class, 0, Vec::new()))
            }
            Some(0x03) => {
                let bits = (&body[1..]).to_vec();
                let nbits = (bits.len() * 8) - (body[0] as usize);
                result.push(ASN1Block::BitString(class, nbits, bits));
            }
            // OCTET STRING
            Some(0x04) => {
                result.push(ASN1Block::OctetString(class, body.to_vec()));
            }
            // NULL
            Some(0x05) => {
                result.push(ASN1Block::Null(class));
            }
            // OBJECT IDENTIFIER
            Some(0x06) => {
                let mut value1 = BigUint::zero();
                let mut value2 = BigUint::from_u8(body[0]).unwrap();
                let mut oidres = Vec::new();
                let mut bindex = 1;

                if body[0] >= 40 {
                    if body[0] < 80 {
                        value1 = BigUint::one();
                        value2 = value2 - BigUint::from_u8(40).unwrap();
                    } else {
                        value1 = BigUint::from_u8(2).unwrap();
                        value2 = value2 - BigUint::from_u8(80).unwrap();
                    }
                }

                oidres.push(value1);
                oidres.push(value2);
                while bindex < body.len() {
                    oidres.push(decode_base127(body, &mut bindex));
                }

                result.push(ASN1Block::ObjectIdentifier(class, oidres));
            }
            // UTF8STRING
            Some(0x0C) => {
                match String::from_utf8(body.to_vec()) {
                    Ok(v) =>
                        result.push(ASN1Block::UTF8String(class, v)),
                    Err(_) =>
                        return Err(ASN1DecodeErr::UTF8DecodeFailure)
                }
            }
            // SEQUENCE
            Some(0x10) => {
                match from_der(body) {
                    Ok(items) =>
                        result.push(ASN1Block::Sequence(class, items)),
                    Err(e) =>
                        return Err(e)
                }
            }
            // SET
            Some(0x11) => {
                match from_der(body) {
                    Ok(items) =>
                        result.push(ASN1Block::Set(class, items)),
                    Err(e) =>
                        return Err(e)
                }
            }
            // IA5 (ASCII) STRING
            Some(0x16) => {
                let val = body.iter().map(|x| *x as char);
                result.push(ASN1Block::IA5String(class, String::from_iter(val)))
            }
            // Dunno.
            _ => {
                result.push(ASN1Block::Unknown(class, tag, body.to_vec()));
            }
        }
        index += len;
    }

    if result.is_empty() {
        Err(ASN1DecodeErr::EmptyBuffer)
    } else {
        Ok(result)
    }
}

fn decode_tag(i: &[u8], index: &mut usize) -> (BigUint, ASN1Class) {
    let tagbyte = i[*index];
    let class   = decode_class(tagbyte);
    let basetag = tagbyte & 0b11111;

    *index += 1;
    if basetag == 0b11111 {
        let res = decode_base127(i, index);
        (res, class)
    } else {
        (BigUint::from(basetag), class)
    }
}

fn decode_base127(i: &[u8], index: &mut usize) -> BigUint {
    let mut res = BigUint::zero();

    loop {
        let nextbyte = i[*index];

        *index += 1;
        res = (res << 7) + BigUint::from(nextbyte & 0x7f);
        if (nextbyte & 0x80) == 0 {
            return res;
        }
    }
}

fn decode_class(i: u8) -> ASN1Class {
    match i >> 6 {
        0b00 => ASN1Class::Universal,
        0b01 => ASN1Class::Application,
        0b10 => ASN1Class::ContextSpecific,
        0b11 => ASN1Class::Private,
        _    => panic!("The universe is broken.")
    }
}

fn decode_length(i: &[u8], index: &mut usize) -> Result<usize,ASN1DecodeErr> {
    let startbyte = i[*index];

    // NOTE: Technically, this size can be much larger than a usize.
    // However, our whole universe starts to break down if we get
    // things that big. So we're boring, and only accept lengths
    // that fit within a usize.
    *index += 1;
    if startbyte >= 0x80 {
        let mut lenlen = (startbyte & 0x7f) as usize;
        let mut res = 0;

        if lenlen > size_of::<usize>() {
            return Err(ASN1DecodeErr::LengthTooLarge);
        }

        while lenlen > 0 {
            res = (res << 8) + (i[*index] as usize);

            *index += 1;
            lenlen -= 1;
        }

        Ok(res)
    } else {
        Ok(startbyte as usize)
    }
}

pub fn to_der(i: &ASN1Block) -> Result<Vec<u8>,ASN1EncodeErr> {
    match i {
        // BOOLEAN
        &ASN1Block::Boolean(cl, val) => {
            let inttag = BigUint::one();
            let mut tagbytes = encode_tag(cl, &inttag);
            tagbytes.push(1);
            tagbytes.push(if val { 0xFF } else { 0x00 });
            Ok(tagbytes)
        }
        // INTEGER
        &ASN1Block::Integer(cl, ref int) => {
            let mut base = int.to_signed_bytes_be();
            let mut lenbytes = encode_len(base.len());
            let     inttag   = BigUint::from_u8(0x02).unwrap();
            let mut tagbytes = encode_tag(cl, &inttag);

            let mut result = Vec::new();
            result.append(&mut tagbytes);
            result.append(&mut lenbytes);
            result.append(&mut base);
            Ok(result)
        }
        // BIT STRING
        &ASN1Block::BitString(cl, bits, ref vs) => {
            let inttag = BigUint::from_u8(0x03).unwrap();
            let mut tagbytes = encode_tag(cl, &inttag);

            if bits == 0 {
                tagbytes.push(0);
                Ok(tagbytes)
            } else {
                let mut lenbytes = encode_len(vs.len() + 1);
                let     nbits    = (vs.len() * 8) - bits;

                let mut result = Vec::new();
                result.append(&mut tagbytes);
                result.append(&mut lenbytes);
                result.push(nbits as u8);
                result.extend(vs.iter());
                Ok(result)
            }
        }
        // OCTET STRING
        &ASN1Block::OctetString(cl, ref bytes) => {
            let inttag = BigUint::from_u8(0x04).unwrap();
            let mut tagbytes = encode_tag(cl, &inttag);
            let mut lenbytes = encode_len(bytes.len());

            let mut result = Vec::new();
            result.append(&mut tagbytes);
            result.append(&mut lenbytes);
            result.extend(bytes.iter());
            Ok(result)
        }
        // NULL
        &ASN1Block::Null(cl) => {
            let inttag = BigUint::from_u8(0x05).unwrap();
            let mut result = encode_tag(cl, &inttag);
            result.push(0);
            Ok(result)
        }
        // OBJECT IDENTIFIER
        &ASN1Block::ObjectIdentifier(cl, ref nums) => {
            match (nums.get(0), nums.get(1)) {
                (Some(v1), Some(v2)) => {
                    let two = BigUint::from_u8(2).unwrap();

                    // first, validate that the first two items meet spec
                    if v1 > &two {
                        return Err(ASN1EncodeErr::ObjectIdentVal1TooLarge)
                    }

                    let u175 = BigUint::from_u8(175).unwrap();
                    let u39 = BigUint::from_u8(39).unwrap();
                    let bound = if v1 == &two { u175 } else { u39 };

                    if v2 > &bound {
                        return Err(ASN1EncodeErr::ObjectIdentVal2TooLarge);
                    }

                    // the following unwraps must be safe, based on the 
                    // validation above.
                    let value1 = v1.to_u8().unwrap();
                    let value2 = v2.to_u8().unwrap();
                    let byte1  = (value1 * 40) + value2;

                    // now we can build all the rest of the body
                    let mut body = vec![byte1];
                    for num in nums.iter().skip(2) {
                        let mut local = encode_base127(&num);
                        body.append(&mut local);
                    }

                    // now that we have the body, we can build the header
                    let inttag = BigUint::from_u8(0x06).unwrap();
                    let mut result = encode_tag(cl, &inttag);
                    let mut lenbytes = encode_len(body.len());

                    result.append(&mut lenbytes);
                    result.append(&mut body);

                    Ok(result)
                }
                _ => {
                    Err(ASN1EncodeErr::ObjectIdentHasTooFewFields)
                }
            }
        }
        // SEQUENCE
        &ASN1Block::Sequence(cl, ref items) => {
            let mut body = Vec::new();

            // put all the subsequences into a block
            for x in items.iter() {
                let mut bytes = to_der(x)?;
                body.append(&mut bytes);
            }

            let inttag = BigUint::from_u8(0x10).unwrap();
            let mut lenbytes = encode_len(body.len());
            let mut tagbytes = encode_tag(cl, &inttag);

            let mut res = Vec::new();
            res.append(&mut tagbytes);
            res.append(&mut lenbytes);
            res.append(&mut body);
            Ok(res)
        }
        // SET
        &ASN1Block::Set(cl, ref items) => {
            let mut body = Vec::new();

            // put all the subsequences into a block
            for x in items.iter() {
                let mut bytes = to_der(x)?;
                body.append(&mut bytes);
            }

            let inttag = BigUint::from_u8(0x11).unwrap();
            let mut lenbytes = encode_len(body.len());
            let mut tagbytes = encode_tag(cl, &inttag);

            let mut res = Vec::new();
            res.append(&mut tagbytes);
            res.append(&mut lenbytes);
            res.append(&mut body);
            Ok(res)
        }
        // IA5String
        &ASN1Block::IA5String(cl, ref str) => {
            let mut body = Vec::new();

            for c in str.chars() {
                body.push(c as u8);
            }

            let inttag = BigUint::from_u8(0x16).unwrap();
            let mut lenbytes = encode_len(body.len());
            let mut tagbytes = encode_tag(cl, &inttag);

            let mut res = Vec::new();
            res.append(&mut tagbytes);
            res.append(&mut lenbytes);
            res.append(&mut body);
            Ok(res)
        }
        // UTF8STRING
        &ASN1Block::UTF8String(cl, ref val) => {
            let mut body = val.clone().into_bytes();
            let inttag = BigUint::from_u8(0x0C).unwrap();
            let mut lenbytes = encode_len(body.len());
            let mut tagbytes = encode_tag(cl, &inttag);

            let mut res = Vec::new();
            res.append(&mut tagbytes);
            res.append(&mut lenbytes);
            res.append(&mut body);
            Ok(res)
        }
        // Unknown blocks
        &ASN1Block::Unknown(class, ref tag, ref bytes) => {
            let mut tagbytes = encode_tag(class, &tag);
            let mut lenbytes = encode_len(bytes.len());

            let mut res = Vec::new();
            res.append(&mut tagbytes);
            res.append(&mut lenbytes);
            res.extend(bytes.iter());
            Ok(res)
        }
    }
}

fn encode_tag(c: ASN1Class, t: &BigUint) -> Vec<u8> {
    let cbyte = encode_class(c);

    match t.to_u8() {
        Some(x) if x < 31 => {
            vec![cbyte | x]
        }
        _ => {
            let mut res = encode_base127(t);
            res.insert(0, cbyte | 0b00011111);
            res
        }
    }
}

fn encode_base127(v: &BigUint) -> Vec<u8> {
    let mut acc = v.clone();
    let mut res = Vec::new();
    let u128 = BigUint::from_u8(128).unwrap();
    let zero = BigUint::zero();

    while acc > zero {
        // we build this vector backwards
        let digit = &acc % &u128;
        acc = acc >> 7;

        match digit.to_u8() {
            None =>
                panic!("7 bits don't fit into 8, cause ..."),
            Some(x) if res.is_empty() =>
                res.push(x),
            Some(x) =>
                res.push(x | 0x80)
        }
    }

    res.reverse();
    res
}

fn encode_class(c: ASN1Class) -> u8 {
    match c {
        ASN1Class::Universal       => 0b00000000,
        ASN1Class::Application     => 0b01000000,
        ASN1Class::ContextSpecific => 0b10000000,
        ASN1Class::Private         => 0b11000000,
    }
}


fn encode_len(x: usize) -> Vec<u8> {
    if x < 128 {
        vec![x as u8]
    } else {
        let mut bstr = Vec::new();
        let mut work = x;

        // convert this into bytes, backwards
        while work > 0 {
            bstr.push(work as u8);
            work = work >> 8;
        }

        // encode the front of the length
        let len = bstr.len() as u8;
        bstr.push(len | 0x80);

        // and then reverse it into the right order
        bstr.reverse();
        bstr
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::{Arbitrary,Gen};
    use std::fs::File;
    use std::io::Read;
    use super::*;

    impl Arbitrary for ASN1Class {
        fn arbitrary<G: Gen>(g: &mut G) -> ASN1Class {
            match g.gen::<u8>() % 4 {
                0 => ASN1Class::Private,
                1 => ASN1Class::ContextSpecific,
                2 => ASN1Class::Universal,
                3 => ASN1Class::Application,
                _ => panic!("I weep for a broken life.")
            }
        }
    }

    quickcheck! {
        fn class_encdec_roundtrips(c: ASN1Class) -> bool {
            c == decode_class(encode_class(c.clone()))
        }

        fn class_decenc_roundtrips(v: u8) -> bool {
            (v & 0b11000000) == encode_class(decode_class(v))
        }
    }

    #[derive(Clone,Debug)]
    struct RandomUint {
        x: BigUint
    }

    impl Arbitrary for RandomUint {
        fn arbitrary<G: Gen>(g: &mut G) -> RandomUint {
            let v = BigUint::from_u32(g.gen::<u32>()).unwrap();
            RandomUint{ x: v }
        }
    }

   quickcheck! {
        fn tags_encdec_roundtrips(c: ASN1Class, t: RandomUint) -> bool {
            let bytes = encode_tag(c, &t.x);
            let mut zero = 0;
            let (t2, c2) = decode_tag(&bytes[..], &mut zero);
            (c == c2) && (t.x == t2)
        }

        fn len_encdec_roundtrips(l: usize) -> bool {
            let bytes = encode_len(l);
            let mut zero = 0;
            match decode_length(&bytes[..], &mut zero) {
                Err(_) => false,
                Ok(l2) => l == l2
            }
        }
    }

    #[derive(Clone,Debug)]
    struct RandomInt {
        x: BigInt
    }

    impl Arbitrary for RandomInt {
        fn arbitrary<G: Gen>(g: &mut G) -> RandomInt {
            let v = BigInt::from_i64(g.gen::<i64>()).unwrap();
            RandomInt{ x: v }
        }
    }

    type ASN1BlockGen<G: Gen> = fn(&mut G, usize) -> ASN1Block;

    fn arb_boolean<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let c = ASN1Class::arbitrary(g);
        let v = g.gen::<bool>();
        ASN1Block::Boolean(c, v)
    }

    fn arb_integer<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let c = ASN1Class::arbitrary(g);
        let d = RandomInt::arbitrary(g);
        ASN1Block::Integer(c, d.x)
    }

    fn arb_bitstr<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let size = g.gen::<u16>() as usize % 16;
        let maxbits = (size as usize) * 8;
        let modbits = g.gen::<u8>() as usize % 8;
        let nbits = if modbits > maxbits
                      { maxbits }
                    else { maxbits - modbits };
        let bytes = g.gen_iter::<u8>().take(size).collect();
        ASN1Block::BitString(class, nbits, bytes)
    }

    fn arb_octstr<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let size = g.gen::<u16>() as usize % 16;
        let bytes = g.gen_iter::<u8>().take(size).collect();
        ASN1Block::OctetString(class, bytes)
    }

    fn arb_null<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        ASN1Block::Null(class)
    }

    fn arb_objid<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let     class = ASN1Class::arbitrary(g);
        let     count = g.gen_range::<usize>(0, 40);
        let     val1  = g.gen::<u8>() % 3;
        let     v2mod = if val1 == 2 { 176 } else { 40 };
        let     val2  = g.gen::<u8>() % v2mod;
        let     v1    = BigUint::from_u8(val1).unwrap();
        let     v2    = BigUint::from_u8(val2).unwrap();
        let mut nums  = vec![v1, v2];

        for _ in 0..count {
            let num = RandomUint::arbitrary(g);
            nums.push(num.x);
        }

        ASN1Block::ObjectIdentifier(class, nums)
    }

    fn arb_seq<G: Gen>(g: &mut G, d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let count = g.gen_range::<usize>(1, 64);
        let mut items = Vec::new();

        for _ in 0..count {
            items.push(limited_arbitrary(g, d - 1));
        }

        ASN1Block::Sequence(class, items)
    }

    fn arb_set<G: Gen>(g: &mut G, d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let count = g.gen_range::<usize>(1, 64);
        let mut items = Vec::new();

        for _ in 0..count {
            items.push(limited_arbitrary(g, d - 1));
        }

        ASN1Block::Set(class, items)
    }

    fn arb_ia5<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let count = g.gen_range::<usize>(0, 384);
        let mut items = Vec::new();

        for _ in 0..count {
            items.push(g.gen::<u8>() as char);
        }

        ASN1Block::IA5String(class, String::from_iter(items.iter()))
    }

    fn arb_utf8<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let val = String::arbitrary(g);
        ASN1Block::UTF8String(class, val)
    }

    fn arb_unknown<G: Gen>(g: &mut G, _d: usize) -> ASN1Block {
        let class = ASN1Class::arbitrary(g);
        let tag   = RandomUint::arbitrary(g);
        let size  = g.gen_range::<usize>(0, 128);
        let items = g.gen_iter::<u8>().take(size).collect();

        ASN1Block::Unknown(class, tag.x, items)
    }

    fn limited_arbitrary<G: Gen>(g: &mut G, d: usize) -> ASN1Block {
        let mut possibles: Vec<ASN1BlockGen<G>> =
            vec![arb_boolean,
                 arb_integer,
                 arb_bitstr,
                 arb_octstr,
                 arb_null,
                 arb_objid,
                 arb_ia5,
                 arb_utf8,
                 arb_unknown];

        if d > 0 {
            possibles.push(arb_seq);
            possibles.push(arb_set);
        }

        match g.choose(&possibles[..]) {
            Some(f) => f(g, d),
            None    => panic!("Couldn't generate arbitrary value.")
        }
    }

    impl Arbitrary for ASN1Block {
        fn arbitrary<G: Gen>(g: &mut G) -> ASN1Block {
            limited_arbitrary(g, 2)
        }
    }

    quickcheck! {
        fn encode_decode_roundtrips(v: ASN1Block) -> bool {
            match to_der(&v) {
                Err(e) => {
                    println!("Serialization error: {:?}", e);
                    false
                }
                Ok(bytes) =>
                    match from_der(&bytes[..]) {
                        Err(e) => {
                            println!("Parse error: {:?}", e);
                            false
                        }
                        Ok(ref rvec) if rvec.len() == 1 => {
                            let v2 = rvec.get(0).unwrap();
                            if &v != v2 {
                                println!("Original: {:?}", v);
                                println!("Constructed: {:?}", v2);
                            }
                            &v == v2
                        }
                        Ok(_) => {
                            println!("Too many results returned.");
                            false
                        }
                    }
            }
        }
    }

    fn result_int(v: i16) -> Result<Vec<ASN1Block>,ASN1DecodeErr> {
        let val = BigInt::from(v);
        Ok(vec![ASN1Block::Integer(ASN1Class::Universal, val)])
    }

    #[test]
    fn base_integer_tests() {
        assert_eq!(from_der(&vec![0x02,0x01,0x00]), result_int(0));
        assert_eq!(from_der(&vec![0x02,0x01,0x7F]), result_int(127));
        assert_eq!(from_der(&vec![0x02,0x02,0x00,0x80]), result_int(128));
        assert_eq!(from_der(&vec![0x02,0x02,0x01,0x00]), result_int(256));
        assert_eq!(from_der(&vec![0x02,0x01,0x80]), result_int(-128));
        assert_eq!(from_der(&vec![0x02,0x02,0xFF,0x7F]), result_int(-129));
    }

    fn can_parse(f: &str) -> Result<Vec<ASN1Block>,ASN1DecodeErr> {
        let mut fd = File::open(f).unwrap();
        let mut buffer = Vec::new();
        fd.read_to_end(&mut buffer);
        from_der(&buffer[..])
    }

    #[test]
    fn x509_tests() {
        assert!(can_parse("test/server.bin").is_ok());
        assert!(can_parse("test/key.bin").is_ok());
    }
}