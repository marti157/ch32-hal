//! Digital to Analog Converter (DAC)
#![macro_use]

use core::marker::PhantomData;

use crate::dma::NoDma;
pub use crate::pac::dac::vals::TrigSel as TriggerSel;
use crate::peripheral::RccPeripheral;
use crate::{into_ref, peripherals, Peripheral, PeripheralRef};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Single 8 or 12 bit value that can be output by the DAC.
///
/// 12-bit values outside the permitted range are silently truncated.
pub enum Value {
    /// 8 bit value
    Bit8(u8),
    /// 12 bit value stored in a u16, left-aligned
    Bit12Left(u16),
    /// 12 bit value stored in a u16, right-aligned
    Bit12Right(u16),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Dual 8 or 12 bit values that can be output by the DAC channels 1 and 2 simultaneously.
///
/// 12-bit values outside the permitted range are silently truncated.
pub enum DualValue {
    /// 8 bit value
    Bit8(u8, u8),
    /// 12 bit value stored in a u16, left-aligned
    Bit12Left(u16, u16),
    /// 12 bit value stored in a u16, right-aligned
    Bit12Right(u16, u16),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Array variant of [`Value`].
pub enum ValueArray<'a> {
    /// 8 bit values
    Bit8(&'a [u8]),
    /// 12 bit value stored in a u16, left-aligned
    Bit12Left(&'a [u16]),
    /// 12 bit values stored in a u16, right-aligned
    Bit12Right(&'a [u16]),
}

/// Driver for a single DAC channel.
///
/// If you want to use both channels, either together or independently,
/// create a [`Dac`] first and use it to access each channel.
pub struct DacChannel<'d, T: Instance, const N: u8, DMA = NoDma> {
    phantom: PhantomData<&'d mut T>,
    #[allow(unused)]
    dma: PeripheralRef<'d, DMA>,
}

/// DAC channel 1 type alias.
pub type DacCh1<'d, T, DMA = NoDma> = DacChannel<'d, T, 1, DMA>;
/// DAC channel 2 type alias.
pub type DacCh2<'d, T, DMA = NoDma> = DacChannel<'d, T, 2, DMA>;

impl<'d, T: Instance, const N: u8, DMA> DacChannel<'d, T, N, DMA> {
    const IDX: usize = (N - 1) as usize;

    /// Create a new `DacChannel` instance, consuming the underlying DAC peripheral.
    ///
    /// If you're not using DMA, pass [`dma::NoDma`] for the `dma` argument.
    ///
    /// The channel is enabled on creation and begin to drive the output pin.
    /// Note that some methods, such as `set_trigger()` and `set_mode()`, will
    /// disable the channel; you must re-enable it with `enable()`.
    ///
    /// By default, triggering is disabled, but it can be enabled using
    /// [`DacChannel::set_trigger()`].
    pub fn new(
        _peri: impl Peripheral<P = T> + 'd,
        dma: impl Peripheral<P = DMA> + 'd,
        pin: impl Peripheral<P = impl DacPin<T, N> + crate::gpio::Pin> + 'd,
    ) -> Self {
        into_ref!(dma, pin);
        pin.set_as_analog();
        T::enable_and_reset();
        let mut dac = Self {
            phantom: PhantomData,
            dma,
        };
        dac.enable();
        dac
    }

    /// Enable or disable this channel.
    pub fn set_enable(&mut self, on: bool) {
        critical_section::with(|_| {
            T::regs().cr().modify(|reg| {
                reg.set_en(Self::IDX, on);
            });
        });
    }

    /// Enable this channel.
    pub fn enable(&mut self) {
        self.set_enable(true)
    }

    /// Disable this channel.
    pub fn disable(&mut self) {
        self.set_enable(false)
    }

    /// Set the trigger source for this channel.
    ///
    /// This method disables the channel, so you may need to re-enable afterwards.
    pub fn set_trigger(&mut self, source: TriggerSel) {
        critical_section::with(|_| {
            T::regs().cr().modify(|reg| {
                reg.set_en(Self::IDX, false);
                reg.set_tsel(Self::IDX, source);
            });
        });
    }

    /// Enable or disable triggering for this channel.
    pub fn set_triggering(&mut self, on: bool) {
        critical_section::with(|_| {
            T::regs().cr().modify(|reg| {
                reg.set_ten(Self::IDX, on);
            });
        });
    }

    /// Software trigger this channel.
    pub fn trigger(&mut self) {
        T::regs().swtrigr().write(|reg| {
            reg.set_swtrig(Self::IDX, true);
        });
    }

    /// Write a new value to this channel.
    ///
    /// If triggering is not enabled, the new value is immediately output; otherwise,
    /// it will be output after the next trigger.
    pub fn set(&mut self, value: Value) {
        match value {
            Value::Bit8(v) => T::regs().dhr8r(Self::IDX).write(|reg| reg.set_dhr(v)),
            Value::Bit12Left(v) => T::regs().dhr12l(Self::IDX).write(|reg| reg.set_dhr(v)),
            Value::Bit12Right(v) => T::regs().dhr12r(Self::IDX).write(|reg| reg.set_dhr(v)),
        }
    }

    /// Read the current output value of the DAC.
    pub fn read(&self) -> u16 {
        T::regs().dor(Self::IDX).read().dor()
    }
}

macro_rules! impl_dma_methods {
    ($n:literal, $trait:ident) => {
        impl<'d, T: Instance, DMA> DacChannel<'d, T, $n, DMA>
        where
            DMA: $trait<T>,
        {
            /// Write `data` to this channel via DMA.
            ///
            /// To prevent delays or glitches when outputing a periodic waveform, the `circular`
            /// flag can be set. This configures a circular DMA transfer that continually outputs
            /// `data`. Note that for performance reasons in circular mode the transfer-complete
            /// interrupt is disabled.
            #[cfg(not(gpdma))]
            pub async fn write(&mut self, data: ValueArray<'_>, circular: bool) {
                // Enable DAC and DMA
                T::regs().cr().modify(|w| {
                    w.set_en(Self::IDX, true);
                    w.set_dmaen(Self::IDX, true);
                });

                let tx_request = self.dma.request();
                let dma_channel = &mut self.dma;

                let tx_options = crate::dma::TransferOptions {
                    circular,
                    half_transfer_ir: false,
                    complete_transfer_ir: !circular,
                    ..Default::default()
                };

                // Initiate the correct type of DMA transfer depending on what data is passed
                let tx_f = match data {
                    ValueArray::Bit8(buf) => unsafe {
                        crate::dma::Transfer::new_write(
                            dma_channel,
                            tx_request,
                            buf,
                            T::regs().dhr8r(Self::IDX).as_ptr() as *mut u8,
                            tx_options,
                        )
                    },
                    ValueArray::Bit12Left(buf) => unsafe {
                        crate::dma::Transfer::new_write(
                            dma_channel,
                            tx_request,
                            buf,
                            T::regs().dhr12l(Self::IDX).as_ptr() as *mut u16,
                            tx_options,
                        )
                    },
                    ValueArray::Bit12Right(buf) => unsafe {
                        crate::dma::Transfer::new_write(
                            dma_channel,
                            tx_request,
                            buf,
                            T::regs().dhr12r(Self::IDX).as_ptr() as *mut u16,
                            tx_options,
                        )
                    },
                };

                tx_f.await;

                T::regs().cr().modify(|w| {
                    w.set_en(Self::IDX, false);
                    w.set_dmaen(Self::IDX, false);
                });
            }
        }
    };
}

impl_dma_methods!(1, DacDma1);
impl_dma_methods!(2, DacDma2);

impl<'d, T: Instance, const N: u8, DMA> Drop for DacChannel<'d, T, N, DMA> {
    fn drop(&mut self) {
        T::disable();
    }
}

/// DAC driver.
///
/// Use this struct when you want to use both channels, either together or independently.
///
/// # Example
///
/// ```ignore
/// // Pins may need to be changed for your specific device.
/// let (dac_ch1, dac_ch2) = embassy_stm32::dac::Dac::new(p.DAC1, NoDma, NoDma, p.PA4, p.PA5).split();
/// ```
pub struct Dac<'d, T: Instance, DMACh1 = NoDma, DMACh2 = NoDma> {
    ch1: DacChannel<'d, T, 1, DMACh1>,
    ch2: DacChannel<'d, T, 2, DMACh2>,
}

impl<'d, T: Instance, DMACh1, DMACh2> Dac<'d, T, DMACh1, DMACh2> {
    /// Create a new `Dac` instance, consuming the underlying DAC peripheral.
    ///
    /// This struct allows you to access both channels of the DAC, where available. You can either
    /// call `split()` to obtain separate `DacChannel`s, or use methods on `Dac` to use
    /// the two channels together.
    ///
    /// The channels are enabled on creation and begin to drive their output pins.
    /// Note that some methods, such as `set_trigger()` and `set_mode()`, will
    /// disable the channel; you must re-enable them with `enable()`.
    ///
    /// By default, triggering is disabled, but it can be enabled using the `set_trigger()`
    /// method on the underlying channels.
    pub fn new(
        _peri: impl Peripheral<P = T> + 'd,
        dma_ch1: impl Peripheral<P = DMACh1> + 'd,
        dma_ch2: impl Peripheral<P = DMACh2> + 'd,
        pin_ch1: impl Peripheral<P = impl DacPin<T, 1> + crate::gpio::Pin> + 'd,
        pin_ch2: impl Peripheral<P = impl DacPin<T, 2> + crate::gpio::Pin> + 'd,
    ) -> Self {
        into_ref!(dma_ch1, dma_ch2, pin_ch1, pin_ch2);
        pin_ch1.set_as_analog();
        pin_ch2.set_as_analog();

        // Enable twice to increment the DAC refcount for each channel.
        T::enable_and_reset();
        T::enable_and_reset();

        let mut ch1 = DacCh1 {
            phantom: PhantomData,
            dma: dma_ch1,
        };
        #[cfg(any(dac_v5, dac_v6, dac_v7))]
        ch1.set_hfsel();
        ch1.enable();

        let mut ch2 = DacCh2 {
            phantom: PhantomData,
            dma: dma_ch2,
        };
        #[cfg(any(dac_v5, dac_v6, dac_v7))]
        ch2.set_hfsel();
        ch2.enable();

        Self { ch1, ch2 }
    }

    /// Split this `Dac` into separate channels.
    ///
    /// You can access and move the channels around separately after splitting.
    pub fn split(self) -> (DacCh1<'d, T, DMACh1>, DacCh2<'d, T, DMACh2>) {
        (self.ch1, self.ch2)
    }

    /// Temporarily access channel 1.
    pub fn ch1(&mut self) -> &mut DacCh1<'d, T, DMACh1> {
        &mut self.ch1
    }

    /// Temporarily access channel 2.
    pub fn ch2(&mut self) -> &mut DacCh2<'d, T, DMACh2> {
        &mut self.ch2
    }

    /// Simultaneously update channels 1 and 2 with a new value.
    ///
    /// If triggering is not enabled, the new values are immediately output;
    /// otherwise, they will be output after the next trigger.
    pub fn set(&mut self, values: DualValue) {
        match values {
            DualValue::Bit8(v1, v2) => T::regs().dhr8rd().write(|reg| {
                reg.set_dhr(0, v1);
                reg.set_dhr(1, v2);
            }),
            DualValue::Bit12Left(v1, v2) => T::regs().dhr12ld().write(|reg| {
                reg.set_dhr(0, v1);
                reg.set_dhr(1, v2);
            }),
            DualValue::Bit12Right(v1, v2) => T::regs().dhr12rd().write(|reg| {
                reg.set_dhr(0, v1);
                reg.set_dhr(1, v2);
            }),
        }
    }
}

trait SealedInstance {
    fn regs() -> &'static crate::pac::dac::Dac;
}

/// DAC instance.
#[allow(private_bounds)]
pub trait Instance: SealedInstance + RccPeripheral + 'static {}
dma_trait!(DacDma1, Instance);
dma_trait!(DacDma2, Instance);

/// Marks a pin that can be used with the DAC
pub trait DacPin<T: Instance, const C: u8>: crate::gpio::Pin + 'static {}

foreach_peripheral!(
    (dac, $inst:ident) => {
        impl crate::dac::SealedInstance for peripherals::$inst {
            fn regs() -> &'static crate::pac::dac::Dac {
                &crate::pac::$inst
            }
        }

        impl crate::dac::Instance for peripherals::$inst {}
    };
);

macro_rules! impl_dac_pin {
    ($inst:ident, $pin:ident, $ch:expr) => {
        impl crate::dac::DacPin<peripherals::$inst, $ch> for crate::peripherals::$pin {}
    };
}
