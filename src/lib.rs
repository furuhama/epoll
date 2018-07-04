extern crate nix;

use nix::sys::epoll::*;
use nix::sys::socket::*;
use nix::unistd::close;
use std::collections::HashMap;
use std::os::unix::io::RawFd;

// State enum represents TCP connection status
// Read -> Write -> End
#[derive(Hash, Eq, PartialEq, Debug, Clone, Copy)]
enum State {
    Read,
    Write,
}

pub fn epoll() -> nix::Result<()> {
    let epfd = epoll_create()?;

    // buffer for epoll event
    let mut epoll_events = vec![EpollEvent::empty(); 1024];
    let mut clients: HashMap<RawFd, State> = HashMap::new();

    // fd of connection-waiting socket
    let sockfd = socket(AddressFamily::Inet, SockType::Stream, SockFlag::SOCK_CLOEXEC, SockProtocol::Tcp)?;
    let addr = SockAddr::new_inet(InetAddr::new(IpAddr::new_v4(127, 0, 0, 1), 8080));
    println!("server fd: {}", sockfd);

    // bind sockfd to local address
    bind(sockfd, &addr)?;
    listen(sockfd, 1024)?;

    let mut ev = EpollEvent::new(EpollFlags::EPOLLIN, sockfd as u64);
    epoll_ctl(epfd, EpollOp::EpollCtlAdd, sockfd, &mut ev)?;

    loop {
        // block a thread until Async IO event occurs
        // & `-1` means no timeout
        let nfds = epoll_wait(epfd, &mut epoll_events, -1)?;
        // some events have occured to n file descriptors
        println!("epoll_wait: nfds = {}", nfds);

        for i in 0..nfds {
            // data has file descriptors which got events
            let data = epoll_events[i].data();
            // events is the events fd got
            let events = epoll_events[i].events();
            println!("i: {}, fd: {:?}, events: {:?}", i, data, events);

            let fd = data as i32;

            if fd == sockfd && events == events & EpollFlags::EPOLLIN {
                // create file descriptor to connect with client by accepting socket connection
                let client_fd = accept(sockfd)?;
                println!("  accept client fd: {:?}", client_fd);

                // set client_fd to the targets of epoll (by using EpollCtlAdd)
                // and, wait for it gets read-able
                // (wait for client send http request)
                let mut ev = EpollEvent::new(EpollFlags::EPOLLIN | EpollFlags::EPOLLONESHOT, client_fd as u64);
                epoll_ctl(epfd, EpollOp::EpollCtlAdd, client_fd, &mut ev)?;

                clients.insert(client_fd, State::Read);
                continue;
            }

            if clients.contains_key(&fd) {
                let client_fd = fd;
                let state = *clients.get(&client_fd).unwrap();
                println!("  client_fd: {:?}, state: {:?}. events: {:?}", client_fd, state, events);

                if events == events & EpollFlags::EPOLLIN && state == State::Read {
                    loop {
                        let mut buf: [u8; 64] = [0; 64];
                        let size = recv(client_fd, &mut buf, MsgFlags::empty())?;
                        let req = std::str::from_utf8(&buf).unwrap().to_string();
                        println!("    recv: bug: {:?}, size: {}", req, size);

                        // continue reading until http request ends
                        if !(req.find("\n\n").is_some() || req.find("\r\n\r\n").is_some()) {
                            continue;
                        }

                        // edit epoll watch list
                        // wait for client_fd gets writable status
                        let mut ev = EpollEvent::new(EpollFlags::EPOLLOUT, client_fd as u64);
                        epoll_ctl(epfd, EpollOp::EpollCtlMod, client_fd, &mut ev)?;

                        clients.insert(client_fd as i32, State::Write);
                        break;
                    }
                } else if events == events & EpollFlags::EPOLLOUT && state == State::Write {
                    // ignore keep-alive configuration & close connection
                    let buf = "HTTP/1.1 200 Ok\nConnection: clone\nContent-Type: text/plain\n\nha?\n\n";
                    let size = send(client_fd, buf.as_bytes(), MsgFlags::empty())?;
                    println!("    send: buf: {:?}, size: {}", buf, size);

                    // remove client_fd from epoll watch list
                    epoll_ctl(epfd, EpollOp::EpollCtlDel, client_fd, &mut epoll_events[i])?;

                    // remove client_fd from client list
                    clients.remove(&client_fd);

                    // close TCP connection
                    shutdown(client_fd, Shutdown::Both)?;
                    close(client_fd)?;
                }
                continue;
            }
        }
    }
}
