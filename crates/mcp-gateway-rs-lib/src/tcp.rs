use std::net::SocketAddr;

use tokio::net::{TcpListener, TcpSocket};

pub struct Tcp {
    address: SocketAddr,
}

impl Tcp {
    pub fn new(address: SocketAddr) -> Self {
        Self { address }
    }
}

impl TryInto<TcpListener> for Tcp {
    type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

    fn try_into(self) -> Result<TcpListener, Self::Error> {
        let address = self.address;
        let socket = if address.is_ipv4() { TcpSocket::new_v4()? } else { TcpSocket::new_v6()? };
        socket.set_reuseaddr(true)?;
        socket.set_keepalive(true)?;
        #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
        socket.set_reuseport(true)?;
        socket.bind(address)?;
        Ok(socket.listen(1024)?)
    }
}
