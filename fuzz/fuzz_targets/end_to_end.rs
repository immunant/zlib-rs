#![no_main]
use libfuzzer_sys::fuzz_target;

use zlib_rs::deflate::DeflateConfig;
use zlib_rs::inflate::InflateConfig;
use zlib_rs::{Flush, ReturnCode};

fuzz_target!(|input: (String, DeflateConfig)| {
    let (data, config) = input;

    if !(0..=15).contains(&config.window_bits) {
        // lower is raw, higher is gzip
        return;
    }

    // only proceed if this is a valid config
    let mut stream = libz_rs_sys::z_stream::default();
    let err = zlib_rs::deflate::init(&mut stream, config);
    if err != ReturnCode::Ok {
        return;
    }
    unsafe { libz_rs_sys::deflateEnd(&mut stream) };

    let length = 8 * 1024;

    // first, deflate the data using the standard zlib
    let mut deflated_rs = vec![0; length as usize];
    let (deflated_rs, error) =
        zlib_rs::deflate::compress_slice(&mut deflated_rs, data.as_bytes(), config);

    let error = ReturnCode::from(error as i32);
    assert_eq!(ReturnCode::Ok, error);

    let mut deflated_ng = vec![0; length as usize];
    let (deflated_ng, error) = compress_slice_ng(&mut deflated_ng, data.as_bytes(), config);

    let error = ReturnCode::from(error as i32);
    assert_eq!(ReturnCode::Ok, error);

    assert_eq!(deflated_rs, deflated_ng);

    let config = InflateConfig {
        window_bits: match config.window_bits {
            8 => 9,
            n => n,
        },
    };

    let mut dest_vec_ng = vec![0u8; data.len()];
    let (output_ng, error) = uncompress_slice_ng(&mut dest_vec_ng, &deflated_rs, config);
    assert_eq!(ReturnCode::Ok, error);

    assert_eq!(
        output_ng,
        data.as_bytes(),
        "zlib-ng cannot decode these bytes!"
    );

    let mut dest_vec_rs = vec![0u8; data.len()];
    let (output_rs, error) =
        zlib_rs::inflate::uncompress_slice(&mut dest_vec_rs, &deflated_rs, config);

    if error != ReturnCode::Ok || output_rs != data.as_bytes() {
        let path = std::env::temp_dir().join("deflate.txt");
        std::fs::write(&path, &data).unwrap();
        let path = std::env::temp_dir().join("inflate.txt");
        std::fs::write(&path, &deflated_rs).unwrap();
        eprintln!("saved deflated file to {path:?}");
    }

    assert_eq!(ReturnCode::Ok, error);
    assert_eq!(output_rs, data.as_bytes());
});

fn compress_slice_ng<'a>(
    output: &'a mut [u8],
    input: &[u8],
    config: DeflateConfig,
) -> (&'a mut [u8], ReturnCode) {
    let DeflateConfig {
        level,
        method,
        window_bits,
        mem_level,
        strategy,
    } = config;

    let mut stream = libz_ng_sys::z_stream {
        next_in: input.as_ptr() as *mut u8,
        avail_in: input.len() as _,
        total_in: 0,
        next_out: output.as_mut_ptr(),
        avail_out: output.len() as _,
        total_out: 0,
        msg: std::ptr::null_mut(),
        state: std::ptr::null_mut(),
        zalloc: ::zlib_rs::allocate::zcalloc,
        zfree: ::zlib_rs::allocate::zcfree,
        opaque: std::ptr::null_mut(),
        data_type: 0,
        adler: 0,
        reserved: 0,
    };

    unsafe {
        let err = libz_ng_sys::deflateInit2_(
            &mut stream,
            level,
            method as i32,
            window_bits as i32,
            mem_level,
            strategy as i32,
            b"1.3.0\0".as_ptr() as *const i8,
            std::mem::size_of::<libz_ng_sys::z_stream>() as i32,
        );
        let return_code = ReturnCode::from(err);

        if return_code != ReturnCode::Ok {
            return (&mut [], return_code);
        }
    };

    let error = unsafe { libz_ng_sys::deflate(&mut stream, Flush::Finish as _) };

    let error: ReturnCode = ReturnCode::from(error as i32);
    assert_eq!(ReturnCode::StreamEnd, error);

    unsafe {
        let err = libz_ng_sys::deflateEnd(&mut stream);
        let return_code: ReturnCode = ReturnCode::from(err);
        assert_eq!(ReturnCode::Ok, return_code);
    }

    (&mut output[..stream.total_out as usize], ReturnCode::Ok)
}

fn uncompress_slice_ng<'a>(
    output: &'a mut [u8],
    input: &[u8],
    config: InflateConfig,
) -> (&'a mut [u8], ReturnCode) {
    let mut stream = libz_ng_sys::z_stream {
        next_in: input.as_ptr() as *mut u8,
        avail_in: input.len() as _,
        total_in: 0,
        next_out: output.as_mut_ptr(),
        avail_out: output.len() as _,
        total_out: 0,
        msg: std::ptr::null_mut(),
        state: std::ptr::null_mut(),
        zalloc: ::zlib_rs::allocate::zcalloc,
        zfree: ::zlib_rs::allocate::zcfree,
        opaque: std::ptr::null_mut(),
        data_type: 0,
        adler: 0,
        reserved: 0,
    };

    let dest_len = output.len();
    let mut dest_len_ptr = 0;

    // z_uintmax_t len, left;
    let mut left;
    let dest;
    let buf: &mut [u8] = &mut [1]; /* for detection of incomplete stream when *destLen == 0 */

    let mut len = input.len() as u64;
    if dest_len != 0 {
        left = dest_len as u64;
        dest_len_ptr = 0;
        dest = output.as_mut_ptr();
    } else {
        left = 1;
        dest = buf.as_mut_ptr();
    }

    let err = unsafe {
        libz_ng_sys::inflateInit2_(
            &mut stream,
            config.window_bits,
            b"1.3.0\0".as_ptr() as *const i8,
            std::mem::size_of::<libz_ng_sys::z_stream>() as i32,
        )
    };
    if err != ReturnCode::Ok as _ {
        return (&mut [], ReturnCode::from(err));
    }

    stream.next_out = dest;
    stream.avail_out = 0;

    let err = loop {
        if stream.avail_out == 0 {
            stream.avail_out = Ord::min(left, u32::MAX as u64) as u32;
            left -= stream.avail_out as u64;
        }

        if stream.avail_out == 0 {
            stream.avail_in = Ord::min(len, u32::MAX as u64) as u32;
            len -= stream.avail_in as u64;
        }

        let err = unsafe { libz_ng_sys::inflate(&mut stream, Flush::NoFlush as _) };
        let err = ReturnCode::from(err);

        if err != ReturnCode::Ok as _ {
            break err;
        }
    };

    if dest_len != 0 {
        dest_len_ptr = stream.total_out;
    } else if stream.total_out != 0 && err == ReturnCode::BufError as _ {
        left = 1;
    }

    unsafe { libz_ng_sys::inflateEnd(&mut stream) };

    let ret = match err {
        ReturnCode::StreamEnd => ReturnCode::Ok,
        ReturnCode::NeedDict => ReturnCode::DataError,
        ReturnCode::BufError if (left + stream.avail_out as u64) != 0 => ReturnCode::DataError,
        _ => err,
    };

    // SAFETY: we have now initialized these bytes
    let output_slice = unsafe {
        std::slice::from_raw_parts_mut(output.as_mut_ptr() as *mut u8, dest_len_ptr as usize)
    };

    (output_slice, ret)
}
