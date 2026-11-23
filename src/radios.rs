//! Radio state through WinRT `Windows.Devices.Radios` (documented API).
//! COM must be initialized on the calling thread before [`Radios::load`].

use std::future::IntoFuture;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::core::{Result, RuntimeType};
use windows_future::IAsyncOperation;

struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Waits for a WinRT async operation without polling. The completion handler
/// wakes the parked thread; the timeout also covers a completion that races
/// between `poll` and `park`.
fn block<T: RuntimeType + 'static>(op: IAsyncOperation<T>) -> Result<T> {
    let future = op.into_future();
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    // A WinRT operation that never completes must not freeze the UI thread.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                if std::time::Instant::now() >= deadline {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::ERROR_TIMEOUT.to_hresult()));
                }
                std::thread::park_timeout(Duration::from_millis(50));
            }
        }
    }
}

/// One radio as shown in Quick Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioInfo {
    /// Name reported by the radio driver.
    pub name: String,
    /// Wi-Fi, Bluetooth, mobile broadband or FM.
    pub kind: RadioKind,
    /// On, Off, Disabled or Unknown.
    pub state: RadioState,
}

/// State to apply to a radio when airplane mode changes. Mobile broadband and
/// FM are left alone: Windows does not force them with the airplane toggle.
pub fn target_state(kind: RadioKind, airplane: bool) -> Option<RadioState> {
    if airplane {
        Some(RadioState::Off)
    } else if matches!(kind, RadioKind::WiFi | RadioKind::Bluetooth) {
        Some(RadioState::On)
    } else {
        None
    }
}

/// True when the machine has radios and none of them is on. An empty list is
/// not airplane mode: a desktop without radios must not light the tile.
pub fn is_airplane(radios: &[RadioInfo]) -> bool {
    let controlled = radios.iter()
        .filter(|radio| radio.kind != RadioKind::Other)
        .collect::<Vec<_>>();
    !controlled.is_empty() && controlled.iter().all(|radio| radio.state != RadioState::On)
}

/// The live radios. Keeps the WinRT objects so `set` can act on the same device.
pub struct Radios {
    items: Vec<Radio>,
}

impl Radios {
    /// Reads the radio list. Needs `RequestAccessAsync` permission from the OS.
    pub fn load() -> Result<Self> {
        let list = block(Radio::GetRadiosAsync()?)?;
        let mut items = Vec::new();
        for index in 0..list.Size()? {
            items.push(list.GetAt(index)?);
        }
        Ok(Self { items })
    }

    /// Snapshot for drawing tiles.
    pub fn info(&self) -> Result<Vec<RadioInfo>> {
        self.items.iter()
            .map(|radio| {
                Ok(RadioInfo {
                    name: radio.Name()?.to_string(),
                    kind: radio.Kind()?,
                    state: radio.State()?,
                })
            })
            .collect()
    }

    /// Whether the radio at `index` is on.
    pub fn is_on(&self, index: usize) -> Result<bool> {
        Ok(self.items[index].State()? == RadioState::On)
    }

    /// Switches one radio. Returns whether Windows allowed the change.
    pub fn set(&self, index: usize, state: RadioState) -> Result<bool> {
        let allowed = block(self.items[index].SetStateAsync(state)?)?;
        Ok(allowed == RadioAccessStatus::Allowed)
    }

    /// Turns every controlled radio off, or Wi-Fi and Bluetooth back on.
    pub fn set_airplane(&self, airplane: bool) -> Result<()> {
        for (index, radio) in self.items.iter().enumerate() {
            let kind = radio.Kind()?;
            if let Some(state) = target_state(kind, airplane) {
                _ = self.set(index, state);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(kind: RadioKind, state: RadioState) -> RadioInfo {
        RadioInfo { name: String::new(), kind, state }
    }

    #[test]
    fn airplane_turns_every_radio_off() {
        assert_eq!(target_state(RadioKind::WiFi, true), Some(RadioState::Off));
        assert_eq!(target_state(RadioKind::Bluetooth, true), Some(RadioState::Off));
        assert_eq!(target_state(RadioKind::MobileBroadband, true), Some(RadioState::Off));
    }

    #[test]
    fn leaving_airplane_restores_only_wifi_and_bluetooth() {
        assert_eq!(target_state(RadioKind::WiFi, false), Some(RadioState::On));
        assert_eq!(target_state(RadioKind::Bluetooth, false), Some(RadioState::On));
        assert_eq!(target_state(RadioKind::MobileBroadband, false), None);
        assert_eq!(target_state(RadioKind::FM, false), None);
    }

    #[test]
    fn airplane_means_no_controlled_radio_is_on() {
        assert!(!is_airplane(&[]));
        assert!(is_airplane(&[
            info(RadioKind::WiFi, RadioState::Off),
            info(RadioKind::Bluetooth, RadioState::Off),
        ]));
        assert!(!is_airplane(&[
            info(RadioKind::WiFi, RadioState::Off),
            info(RadioKind::Bluetooth, RadioState::On),
        ]));
    }
}
