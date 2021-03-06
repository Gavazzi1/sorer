use sorer::dataframe::*;
use sorer::schema::*;

use std::env;
use std::fs::File;
use std::io::SeekFrom;
use std::io::{BufRead, BufReader, Seek};
use std::thread;

mod clap;
use clap::*;

fn main() {
    // parse the arguments
    let args: Vec<String> = env::args().collect();
    let parsed_args = ProgArgs::from(args);

    // infer the schema
    let f: File = File::open(parsed_args.file.clone()).unwrap();
    let reader = BufReader::new(f);
    let schema = infer_schema(reader);

    // number of threads to use
    let num_threads = 8;

    // the total number of bytes to read
    let num_chars = if parsed_args.len == std::u64::MAX {
        (std::fs::metadata(parsed_args.file.clone()).unwrap().len()
            - parsed_args.from) as f64
    } else {
        parsed_args.len as f64
    };
    // each thread will parse this many characters +- some number
    let step = (num_chars / num_threads as f64).ceil() as u64;

    // setup the work array with the from / len for each thread
    // each element in the work array is a tuple of (starting index, number of byte for this thread)
    let f: File = File::open(parsed_args.file.clone()).unwrap();
    let mut reader2 = BufReader::new(f);
    let mut work: Vec<(u64, u64)> = Vec::with_capacity(num_threads + 1);

    // add the first one separately since we want to access the previous thread's
    // work when in the loop. Since the work of the first thread will call
    // `read_file(schema, 0, step)` it will not throw away the first line
    // since from is 0 and will throw away the last line since step > 0
    work.push((parsed_args.from, step));

    let mut so_far = parsed_args.from;
    let mut buffer = Vec::new();

    // This loop finds the byte offset for the start of a line
    // by adding the length of the last line that a previous thread would've
    // thrown away. The work gets added to the following thread so that
    // each thread starts at a full line and reads only until the end of a line
    for i in 1..num_threads {
        so_far += step;
        // advance the reader to this threads starting index then
        // find the next newline character
        reader2.seek(SeekFrom::Start(so_far)).unwrap();
        reader2.read_until(b'\n', &mut buffer).unwrap();
        work.push((so_far, step));

        // Since the previous thread throws away the last line, add the length
        // of the last line of prev thread to the work of this thread so that
        // we read all lines.
        work.get_mut(i - 1).unwrap().1 += buffer.len() as u64 + 1;
        buffer.clear();
    }

    // initialize the threads with their own BufReader
    let mut threads = Vec::new();
    for w in work {
        let new_schema = schema.clone();
        let f: File = File::open(parsed_args.file.clone()).unwrap();
        let mut r = BufReader::new(f);
        // spawn the thread and give it a closure which calls `from_file`
        // to parse the data into columnar format.
        threads.push(thread::spawn(move || {
            DataFrame::from_file(new_schema, &mut r, w.0, w.1)
        }));
    }

    // initialize the resulting columnar data frame
    let mut parsed_data: Vec<Column> = DataFrame::init(&schema);
    // let all the threads finish then combine the parsed data into the
    // columnar data frame
    for t in threads {
        let mut x: Vec<Column> = t.join().unwrap();
        let iter = parsed_data.iter_mut().zip(x.iter_mut());
        for (complete, partial) in iter {
            match (complete, partial) {
                (Column::Bool(c1), Column::Bool(c2)) => c1.append(c2),
                (Column::Int(c1), Column::Int(c2)) => c1.append(c2),
                (Column::Float(c1), Column::Float(c2)) => c1.append(c2),
                (Column::String(c1), Column::String(c2)) => c1.append(c2),
                _ => panic!("Unexpected result from thread"),
            }
        }
    }

    // metadata about the parsed file
    let num_cols = parsed_data.len() as u64;
    let num_lines = if num_cols != 0 {
        (match &parsed_data[0] {
            Column::Bool(b) => b.len(),
            Column::Int(b) => b.len(),
            Column::Float(b) => b.len(),
            Column::String(b) => b.len(),
        }) as u64
    } else {
        0
    };
    // Retrieve and return the requested data
    match parsed_args.option {
        Options::PrintColIdx(n1, n2) => {
            if n1 >= num_cols {
                println!(
                    "Error: There are only {} fields in the schema",
                    num_cols
                );
            } else if n2 >= num_lines {
                println!("Error: Only {} lines were parsed", num_lines);
            } else {
                println!("{}", parsed_data.get(n1, n2));
            }
        }
        Options::IsMissingIdx(n1, n2) => {
            if n1 >= num_cols {
                println!(
                    "Error: There are only {} fields in the schema",
                    num_cols
                );
            } else if n2 >= num_lines {
                println!("Error: Only {} lines were parsed", num_lines);
            } else {
                if parsed_data.get(n1, n2) == Data::Null {
                    println!("{}", 1);
                } else {
                    println!("{}", 0);
                }
            }
        }
        Options::PrintColType(n) => {
            // note:
            // if argument is `-print_col_type`, we only need to infer the
            // schema, but we currently parse the file anyways so that
            // we dont disregard any -from and -len a arguments.
            // This can be very easily changed by adding a match right after
            // the call to `infer_schema` and returning from main if desired.
            if n >= num_cols {
                println!(
                    "Error: There are only {} fields in the schema",
                    num_cols
                );
            } else {
                println!(
                    "{}",
                    format!("{:?}", &schema[n as usize]).to_uppercase()
                );
            }
        }
    }
}
