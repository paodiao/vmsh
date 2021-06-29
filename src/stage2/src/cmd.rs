use nix::{self, unistd};
use simple_error::try_with;
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;

use crate::procfs;
use crate::pty::Pts;
use crate::result::Result;

pub struct Cmd {
    environment: HashMap<OsString, OsString>,
    command: String,
    arguments: Vec<String>,
    home: Option<OsString>,
    pts: Pts,
}

fn read_environment(pid: unistd::Pid) -> Result<HashMap<OsString, OsString>> {
    let path = procfs::get_path().join(pid.to_string()).join("environ");
    let f = try_with!(File::open(&path), "failed to open {}", path.display());
    let reader = BufReader::new(f);
    let res: HashMap<OsString, OsString> = reader
        .split(b'\0')
        .filter_map(|var| {
            let var = match var {
                Ok(var) => var,
                Err(_) => return None,
            };

            let tuple: Vec<&[u8]> = var.splitn(2, |b| *b == b'=').collect();
            if tuple.len() != 2 {
                return None;
            }
            Some((
                OsString::from_vec(Vec::from(tuple[0])),
                OsString::from_vec(Vec::from(tuple[1])),
            ))
        })
        .collect();
    Ok(res)
}

impl Cmd {
    pub fn new(
        command: Option<String>,
        args: Vec<String>,
        pid: unistd::Pid,
        home: Option<OsString>,
        pts: Pts,
    ) -> Result<Cmd> {
        let arguments = if command.is_none() {
            vec![String::from("-l")]
        } else {
            args
        };

        let command = command.unwrap_or_else(|| String::from("sh"));

        let variables = try_with!(
            read_environment(pid),
            "could not inherit environment variables of container"
        );
        Ok(Cmd {
            command,
            arguments,
            home,
            pts,
            environment: variables,
        })
    }
    pub fn spawn(mut self) -> Result<Child> {
        let default_path =
            OsString::from("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
        self.environment.insert(
            OsString::from("PATH"),
            env::var_os("PATH").unwrap_or(default_path),
        );

        if let Some(path) = self.home {
            self.environment.insert(OsString::from("HOME"), path);
        }

        let pts = self.pts.clone();

        let child = unsafe {
            Command::new(&self.command)
                .args(&self.arguments)
                .envs(self.environment)
                .pre_exec(move || {
                    if let Err(e) = pts.attach() {
                        eprintln!("failed to attach to terminal: {}", e);
                        return Err(io::Error::from_raw_os_error(libc::EINVAL));
                    }
                    Ok(())
                })
                .spawn()
        };
        Ok(try_with!(
            child,
            "failed to spawn {} {}",
            self.command,
            self.arguments.join(" ")
        ))
    }

    // TODO: maybe in future
    //pub fn exec_chroot(self) -> Result<()> {
    //    let err = unsafe {
    //        Command::new(&self.command)
    //            .args(self.arguments)
    //            .envs(self.environment)
    //            .pre_exec(|| {
    //                match unistd::chroot("/var/lib/vmsh") {
    //                    Err(nix::Error::Sys(errno)) => {
    //                        eprintln!(
    //                            "failed to chroot to /var/lib/vmsh: {}",
    //                            nix::Error::Sys(errno)
    //                        );
    //                        return Err(io::Error::from(errno));
    //                    }
    //                    Err(e) => {
    //                        eprintln!("failed to chroot to /var/lib/vmsh: {}", e);
    //                        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    //                    }
    //                    _ => {}
    //                }

    //                if let Err(e) = env::set_current_dir("/") {
    //                    eprintln!("failed to change directory to /");
    //                    return Err(e);
    //                }

    //                Ok(())
    //            })
    //            .exec()
    //    };
    //    try_with!(Err(err), "failed to execute `{}`", self.command);
    //    Ok(())
    //}
}
