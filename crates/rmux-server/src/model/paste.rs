// Ported from tmux paste.c @ 8f25579c
use std::collections::BTreeMap;
use std::rc::Rc;

use super::state::{ModelError, Server, clean_name};
use crate::ids::{Arena, PasteBufferId};
use rmux_util::{
    bytes::{ByteString, cstr},
    utf8,
    vis::VisFlags,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteEvent {
    pub event: &'static str,
    pub name: ByteString,
}

pub struct PasteBuffer {
    pub name: ByteString,
    pub data: Rc<Vec<u8>>,
    pub created: i64,
    pub order: u32,
    pub automatic: bool,
}

#[derive(Default)]
pub struct PasteStore {
    buffers: Arena<PasteBuffer, PasteBufferId>,
    names: BTreeMap<ByteString, PasteBufferId>,
    order: BTreeMap<u32, PasteBufferId>,
    next_index: u32,
    next_order: u32,
    automatic: u32,
}

impl PasteStore {
    pub fn get(&self, id: PasteBufferId) -> Option<&PasteBuffer> {
        self.buffers.get(id)
    }
    pub fn get_name(&self, name: &[u8]) -> Option<PasteBufferId> {
        if cstr(name).is_empty() {
            return None;
        }
        self.names.get(cstr(name)).copied()
    }
    pub fn walk(&self, previous: Option<PasteBufferId>) -> Option<PasteBufferId> {
        match previous {
            None => self.order.last_key_value().map(|(_, id)| *id),
            Some(id) => self
                .order
                .range(..self.get(id)?.order)
                .next_back()
                .map(|(_, id)| *id),
        }
    }
    pub fn top(&self) -> Option<PasteBufferId> {
        self.order
            .values()
            .rev()
            .copied()
            .find(|id| self.get(*id).is_some_and(|b| b.automatic))
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn automatic_count(&self) -> u32 {
        self.automatic
    }
    fn insert(
        &mut self,
        name: ByteString,
        data: Vec<u8>,
        created: i64,
        automatic: bool,
    ) -> Result<PasteBufferId, ModelError> {
        let order = self.next_order;
        self.next_order = self.next_order.wrapping_add(1);
        let id = self.buffers.insert(PasteBuffer {
            name: name.clone(),
            data: Rc::new(data),
            created,
            order,
            automatic,
        })?;
        self.names.insert(name, id);
        self.order.insert(order, id);
        self.automatic += u32::from(automatic);
        Ok(id)
    }
}

pub fn paste_get_name(server: &Server, name: &[u8]) -> Option<PasteBufferId> {
    server.paste.get_name(name)
}
pub fn paste_get_top(server: &Server) -> Option<PasteBufferId> {
    server.paste.top()
}
pub fn paste_walk(server: &Server, previous: Option<PasteBufferId>) -> Option<PasteBufferId> {
    server.paste.walk(previous)
}
pub fn paste_is_empty(server: &Server) -> bool {
    server.paste.is_empty()
}
pub fn paste_buffer_name(server: &Server, id: PasteBufferId) -> Option<&[u8]> {
    Some(server.paste.get(id)?.name.as_bytes())
}
pub fn paste_buffer_data(server: &Server, id: PasteBufferId) -> Option<&[u8]> {
    Some(&server.paste.get(id)?.data)
}
pub fn paste_buffer_order(server: &Server, id: PasteBufferId) -> Option<u32> {
    Some(server.paste.get(id)?.order)
}
pub fn paste_buffer_created(server: &Server, id: PasteBufferId) -> Option<i64> {
    Some(server.paste.get(id)?.created)
}

pub fn paste_remove(server: &mut Server, id: PasteBufferId) -> Result<(), ModelError> {
    let name = server
        .paste
        .get(id)
        .ok_or(ModelError::StaleId)?
        .name
        .clone();
    server.fire_paste_event("paste-buffer-deleted", &name);
    let Some(buffer) = server.paste.buffers.request_remove(id)? else {
        return Ok(());
    };
    server.paste.names.remove(buffer.name.as_bytes());
    server.paste.order.remove(&buffer.order);
    server.paste.automatic -= u32::from(buffer.automatic);
    Ok(())
}

pub fn paste_add(
    server: &mut Server,
    prefix: Option<&[u8]>,
    data: Vec<u8>,
    limit: u32,
) -> Result<Option<PasteBufferId>, ModelError> {
    if data.is_empty() {
        return Ok(None);
    }
    while server.paste.automatic >= limit {
        let oldest = server
            .paste
            .order
            .values()
            .copied()
            .find(|id| server.paste.get(*id).is_some_and(|b| b.automatic));
        let Some(oldest) = oldest else {
            break;
        };
        paste_remove(server, oldest)?;
    }
    let name = loop {
        let mut name = cstr(prefix.unwrap_or(b"buffer")).to_vec();
        name.extend_from_slice(server.paste.next_index.to_string().as_bytes());
        server.paste.next_index = server.paste.next_index.wrapping_add(1);
        if server.paste.get_name(&name).is_none() {
            break ByteString(name);
        }
    };
    let id = server
        .paste
        .insert(name.clone(), data, server.current_time.0, true)?;
    server.fire_paste_event("paste-buffer-changed", &name);
    Ok(Some(id))
}

#[derive(Debug)]
pub struct PasteSetError {
    pub error: ModelError,
    pub data: Vec<u8>,
}

pub fn paste_set(
    server: &mut Server,
    data: Vec<u8>,
    name: Option<&[u8]>,
    limit: u32,
) -> Result<Option<PasteBufferId>, PasteSetError> {
    if data.is_empty() {
        return Ok(None);
    }
    let Some(name) = name else {
        return paste_add(server, None, data, limit).map_err(|error| PasteSetError {
            error,
            data: Vec::new(),
        });
    };
    let name = cstr(name);
    if name.is_empty() {
        return Err(PasteSetError {
            error: ModelError::message(b"empty buffer name"),
            data,
        });
    }
    let Some(newname) = clean_name(name, false) else {
        let mut error = b"invalid buffer name: ".to_vec();
        error.extend_from_slice(name);
        return Err(PasteSetError {
            error: ModelError::message(&error),
            data,
        });
    };
    if let Some(old) = server.paste.get_name(&newname) {
        if let Err(error) = paste_remove(server, old) {
            return Err(PasteSetError { error, data });
        }
    }
    let id = server
        .paste
        .insert(newname.clone().into(), data, server.current_time.0, false)
        .map_err(|error| PasteSetError {
            error,
            data: Vec::new(),
        })?;
    server.fire_paste_event("paste-buffer-changed", &newname);
    Ok(Some(id))
}

pub fn paste_rename(
    server: &mut Server,
    oldname: Option<&[u8]>,
    newname: &[u8],
) -> Result<(), ModelError> {
    let oldname = cstr(oldname.unwrap_or(b""));
    if oldname.is_empty() {
        return Err(ModelError::message(b"no buffer"));
    }
    let newname = cstr(newname);
    if newname.is_empty() {
        return Err(ModelError::message(b"new name is empty"));
    }
    let Some(name) = clean_name(newname, false) else {
        let mut error = b"invalid buffer name: ".to_vec();
        error.extend_from_slice(newname);
        return Err(ModelError::message(&error));
    };
    let Some(id) = server.paste.get_name(oldname) else {
        let mut error = b"no buffer ".to_vec();
        error.extend_from_slice(oldname);
        return Err(ModelError::message(&error));
    };
    if let Some(other) = server.paste.get_name(&name) {
        if other == id {
            return Ok(());
        }
        paste_remove(server, other)?;
    }
    server.paste.names.remove(oldname);
    let buffer = server
        .paste
        .buffers
        .get_mut(id)
        .ok_or(ModelError::StaleId)?;
    server.paste.automatic -= u32::from(buffer.automatic);
    buffer.automatic = false;
    buffer.name = name.clone().into();
    server.paste.names.insert(name.clone().into(), id);
    server.fire_paste_event("paste-buffer-deleted", oldname);
    server.fire_paste_event("paste-buffer-changed", &name);
    Ok(())
}

pub fn paste_replace(
    server: &mut Server,
    id: PasteBufferId,
    data: Vec<u8>,
) -> Result<(), ModelError> {
    let buffer = server
        .paste
        .buffers
        .get_mut(id)
        .ok_or(ModelError::StaleId)?;
    buffer.data = Rc::new(data);
    let name = buffer.name.clone();
    server.fire_paste_event("paste-buffer-changed", &name);
    Ok(())
}

pub fn paste_make_sample(buffer: &PasteBuffer, output: &mut Vec<u8>) {
    output.clear();
    utf8::strvis(
        output,
        &buffer.data[..buffer.data.len().min(200)],
        VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL,
    );
    if buffer.data.len() > 200 || output.len() > 200 {
        output.truncate(200);
        output.extend_from_slice(b"...");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_match_pinned_oracle() {
        use std::process::Command;
        let oracle = std::path::Path::new("/Users/j/fun/rmux/oracle/bin/tmux");
        if !oracle.is_file() {
            eprintln!("skipping paste oracle: pinned tmux unavailable");
            return;
        }
        let dir = std::env::temp_dir().join(format!("rmux-paste-oracle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("socket");
        struct Cleanup<'a> {
            oracle: &'a std::path::Path,
            socket: std::path::PathBuf,
            dir: std::path::PathBuf,
        }
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = Command::new(self.oracle)
                    .arg("-S")
                    .arg(&self.socket)
                    .arg("kill-server")
                    .output();
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
        let _cleanup = Cleanup {
            oracle,
            socket: socket.clone(),
            dir: dir.clone(),
        };
        let start = Command::new(oracle)
            .arg("-S")
            .arg(&socket)
            .args([
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "sample",
                "sleep 60",
            ])
            .output()
            .unwrap();
        assert!(
            start.status.success(),
            "{}",
            String::from_utf8_lossy(&start.stderr)
        );
        for data in [
            b"a\0b\t\n\xff".to_vec(),
            vec![b'x'; 200],
            vec![b'x'; 201],
            vec![0; 101],
            "é".repeat(101).into_bytes(),
        ] {
            let path = dir.join("input");
            std::fs::write(&path, &data).unwrap();
            let load = Command::new(oracle)
                .arg("-S")
                .arg(&socket)
                .args(["load-buffer", "-b", "sample"])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                load.status.success(),
                "{}",
                String::from_utf8_lossy(&load.stderr)
            );
            let output = Command::new(oracle)
                .arg("-S")
                .arg(&socket)
                .args(["list-buffers", "-F", "#{buffer_sample}"])
                .output()
                .unwrap();
            assert!(output.status.success());
            let buffer = PasteBuffer {
                name: "sample".into(),
                data: Rc::new(data),
                created: 0,
                order: 0,
                automatic: false,
            };
            let mut actual = Vec::new();
            paste_make_sample(&buffer, &mut actual);
            actual.push(b'\n');
            assert_eq!(actual, output.stdout);
        }
    }
    #[test]
    fn automatic_named_order_and_collision() {
        let mut server = Server::new();
        server.current_time = (7, 0);
        let first = paste_add(&mut server, None, vec![0, 1], 2)
            .unwrap()
            .unwrap();
        let named = paste_set(&mut server, vec![2], Some(b"named"), 2)
            .unwrap()
            .unwrap();
        assert_eq!(paste_get_top(&server), Some(first));
        let second = paste_add(&mut server, None, vec![3], 2).unwrap().unwrap();
        assert_eq!(paste_walk(&server, None), Some(second));
        assert_eq!(paste_walk(&server, Some(second)), Some(named));
        paste_replace(&mut server, first, Vec::new()).unwrap();
        assert_eq!(server.paste.get(first).unwrap().order, 0);
        paste_rename(&mut server, Some(b"buffer0"), b"named").unwrap();
        assert!(server.paste.get(named).is_none());
        assert_eq!(server.paste.get(first).unwrap().created, 7);
        assert_eq!(server.paste.automatic_count(), 1);
        assert_eq!(paste_get_top(&server), Some(second));
        let before = server.effects.len();
        paste_rename(&mut server, Some(b"named"), b"named").unwrap();
        assert_eq!(server.effects.len(), before);
        let third = paste_add(&mut server, None, vec![4], 0).unwrap().unwrap();
        assert!(server.paste.get(second).is_none());
        assert_eq!(server.paste.automatic_count(), 1);
        assert_eq!(paste_get_top(&server), Some(third));
        assert_eq!(paste_walk(&server, Some(third)), Some(first));
        let events = server
            .effects
            .iter()
            .filter_map(|e| match e {
                super::super::state::ModelEffect::Paste(e) => Some((e.event, e.name.as_bytes())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(events.windows(3).any(|e| e
            == [
                ("paste-buffer-deleted", b"named".as_slice()),
                ("paste-buffer-deleted", b"buffer0".as_slice()),
                ("paste-buffer-changed", b"named".as_slice())
            ]));
    }
    #[test]
    fn empty_set_validation_and_ownership() {
        let mut server = Server::new();
        assert!(
            paste_set(&mut server, Vec::new(), Some(b""), 1)
                .unwrap()
                .is_none()
        );
        assert!(
            paste_set(&mut server, Vec::new(), Some(b"\xff"), 1)
                .unwrap()
                .is_none()
        );
        let failure = paste_set(&mut server, vec![1, 0, 2], Some(b"\xff"), 1).unwrap_err();
        assert_eq!(failure.data, [1, 0, 2]);
        assert!(
            matches!(&failure.error, ModelError::Message(m) if m == b"invalid buffer name: \xff")
        );
        assert_eq!(
            paste_rename(&mut server, None, b"x")
                .unwrap_err()
                .to_string(),
            "no buffer"
        );
        assert_eq!(
            paste_rename(&mut server, Some(b"x"), b"")
                .unwrap_err()
                .to_string(),
            "new name is empty"
        );
        assert_eq!(
            paste_rename(&mut server, Some(b"x"), b"y")
                .unwrap_err()
                .to_string(),
            "no buffer x"
        );
        assert_eq!(
            paste_set(&mut server, vec![1], Some(b""), 1)
                .unwrap_err()
                .error
                .to_string(),
            "empty buffer name"
        );
        let named = paste_set(&mut server, vec![0, 1, 0], Some(b"buffer0"), 1)
            .unwrap()
            .unwrap();
        let automatic = paste_add(&mut server, None, vec![1], 1).unwrap().unwrap();
        assert_eq!(
            server.paste.get(automatic).unwrap().name,
            b"buffer1".as_slice()
        );
        assert_eq!(server.paste.get(named).unwrap().data.as_slice(), [0, 1, 0]);
        paste_remove(&mut server, automatic).unwrap();
        assert!(paste_get_top(&server).is_none());
        assert!(!paste_is_empty(&server));
    }
    #[test]
    fn binary_sample_and_boundary() {
        let mut buffer = PasteBuffer {
            name: "x".into(),
            data: Rc::new(b"a\0b\t\n".to_vec()),
            created: 1,
            order: 0,
            automatic: false,
        };
        let mut sample = Vec::new();
        paste_make_sample(&buffer, &mut sample);
        assert_eq!(sample, b"a\\0b\\t\\n");
        buffer.data = Rc::new(vec![b'x'; 200]);
        paste_make_sample(&buffer, &mut sample);
        assert_eq!(sample.len(), 200);
        Rc::make_mut(&mut buffer.data).push(b'x');
        paste_make_sample(&buffer, &mut sample);
        assert_eq!(sample, [vec![b'x'; 200], b"...".to_vec()].concat());
        buffer.data = Rc::new(vec![0; 100]);
        paste_make_sample(&buffer, &mut sample);
        assert_eq!(sample.len(), 200);
        Rc::make_mut(&mut buffer.data).push(0);
        paste_make_sample(&buffer, &mut sample);
        assert_eq!(&sample[200..], b"...");
    }
}
