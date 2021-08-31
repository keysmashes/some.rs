use nix::sys::signal;
use structopt::StructOpt;
use terminal_size::{terminal_size, Height, Width};

use std::{
    env,
    error::Error,
    fs::File,
    io::{self, Read, Write},
    mem,
    path::PathBuf,
    process,
};

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(name = "FILE", parse(from_os_str))]
    filename: Option<PathBuf>,
}

fn lines_used(buf: &[u8], width: usize) -> usize {
    // There are a bunch of different approaches we could take here.
    //
    // The first is just "count how many newlines there are and add one":
    //
    // ```rust
    // bytecount::count(&buf, b'\n') + 1
    // ```
    //
    // but that doesn't account for lines longer than a certain width being wrapped.
    //
    // A somewhat better approach would be to do something along the lines of:
    //
    // ```rust
    // buf.split(|c| *c == b'\n').map(|line| (line.len()-1) / width + 1).sum()
    // ```
    //
    // but this has inaccuracies around double-width characters, and also doesn't account for
    // escape sequences (e.g. those for changing the colour of text).
    //
    // A more comprehensive solution would probably use something like the unicode-width crate to
    // check the length of each line; however, even that would be [imperfect][1], and would
    // probably be significantly slower than the simpler solutions.
    //
    // Ultimately the solution we use accounts for lines wrapping, but does not take into account
    // the possibility that characters may not be displayed in exactly one column. This means that,
    // if double-width characters are used extensively, the pager may not be invoked when it should
    // be, and that conversely, if many escape codes are used that take up no screen space, the
    // pager may be invoked too eagerly. This feels like a reasonable compromise.
    //
    // [1]: https://github.com/unicode-rs/unicode-width/issues/4
    buf.split(|c| *c == b'\n')
        .map(|line| (line.len().saturating_sub(1)) / width + 1)
        .sum()
}

#[cfg(test)]
mod lines_used {
    use super::lines_used;

    #[test]
    fn counts_newlines() {
        assert_eq!(lines_used(b"a", 100), 1);
        assert_eq!(lines_used(b"a\nb", 100), 2);
    }

    #[test]
    fn accounts_for_wrapping() {
        assert_eq!(lines_used(b"aaa", 3), 1);
        assert_eq!(lines_used(b"aaaa", 3), 2);
    }

    #[test]
    fn empty_string_takes_one_line() {
        assert_eq!(lines_used(b"", 100), 1);
    }
}

enum Contents {
    All(Vec<u8>),
    Part(Vec<u8>),
}

/// Reads some prefix of a file, either the whole file or approximately a screen-sized chunk of it.
fn read_prefix(file: &mut dyn Read) -> Result<Contents, (Vec<u8>, Box<dyn Error>)> {
    if let Some((Width(width), Height(height))) = terminal_size() {
        let usable_height = height.saturating_sub(3);
        let mut buf: Vec<u8> = vec![0; (width * usable_height) as usize];
        let mut len = 0;
        while lines_used(&buf[..len], width as usize) <= usable_height as usize {
            match file.read(&mut buf[len..]) {
                Ok(0) => {
                    buf.truncate(len);
                    return Ok(Contents::All(buf));
                }
                Ok(n) => {
                    len += n;
                    if len == buf.len() {
                        // The distinction between length and capacity is in an irritating place;
                        // it would be nice to be able to use Vec's heuristics for increasing
                        // capacity here rather than having to implement our own. In other words,
                        // TODO: this seems likely to be less-than-optimal
                        buf.extend(vec![0; (width * usable_height) as usize]);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(e) => {
                    buf.truncate(len);
                    return Err((buf, Box::new(e)));
                }
            }
        }
        buf.truncate(len);
        Ok(Contents::Part(buf))
    } else {
        // We don't know how big the terminal is, just invoke a pager immediately.
        Ok(Contents::Part(Vec::new()))
    }
}

fn main() {
    let opt = Opt::from_args();
    let mut file: Box<dyn Read> = match opt.filename {
        Some(filename) => Box::new(File::open(filename).expect("Could not open file")),
        None => Box::new(io::stdin()),
    };
    match read_prefix(&mut file) {
        Ok(Contents::All(buf)) => {
            match io::stdout().write_all(&buf) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
                Err(e) => panic!("Could not write output: {:?}", e),
            };
        }
        Ok(Contents::Part(buf)) => {
            let pager = env::var("PAGER")
                .ok()
                .and_then(|pager| shlex::split(&pager))
                .unwrap_or_else(|| vec!["less".to_owned()]);
            // Ignore SIGINT: if someone using less presses ctrl-c, they expect less to handle it.
            // Taking the default action (terminating) would be surprising and unhelpful. If the
            // pager exits on ctrl-c, we'll exit regardless.
            //
            // Note that, by now, we already have a screenful of text to show; if the user is
            // paging from a slow pipe and wishes to stop before a full page of output is received,
            // they can still do that just by pressing ctrl-c.
            unsafe { signal::signal(signal::Signal::SIGINT, signal::SigHandler::SigIgn) }.unwrap();
            let mut command = process::Command::new(&pager[0])
                .args(pager.get(1..).unwrap_or(&[]))
                .stdin(process::Stdio::piped())
                .spawn()
                .expect("Could not start pager");
            let mut stdin = command.stdin.take().unwrap();
            match stdin.write_all(&buf) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return,
                Err(e) => panic!("Could not write to pager: {:?}", e),
            }
            // It would arguably be nice to use io::BufReader here, but it essentially just does
            // what we're doing except with a buffer that's 8KB rather than 16KB.
            let mut buf = vec![0; 16 * 1024];
            loop {
                match file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        match stdin.write_all(&buf[..n]) {
                            Ok(_) => {}
                            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return,
                            Err(e) => panic!("Could not write to pager: {:?}", e),
                        };
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => panic!("Could not read from file: {:?}", e),
                }
            }
            mem::drop(stdin);
            // Since we exit straight away, there's no need to restore the default signal handler.
            process::exit(command.wait().unwrap().code().unwrap_or(1));
        }
        Err((buf, e)) => {
            match io::stdout().write_all(&buf) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return,
                Err(_) => {} // TODO: might be nice to report this as well?
            }
            panic!("Could not read from file: {:?}", e);
        }
    }
}
