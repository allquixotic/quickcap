pub const SAMPLE_RATE: usize = 48_000;
pub const FPS: u32 = 10;
pub const VIDEO_BITRATE: u32 = 1_500_000;

pub fn sample_time(samples: u64) -> i64 { (samples * 10_000_000 / SAMPLE_RATE as u64) as i64 }
pub fn frame_time(index: u64) -> i64 { (index * 10_000_000 / FPS as u64) as i64 }

/// One endpoint's mono, already resampled audio. Late packets are clipped, future
/// packets remain in a bounded queue managed by the caller.
#[derive(Debug)]
pub struct AudioPacket { pub start: u64, pub samples: Vec<f32> }

pub fn mix_packet(output: &mut [f32], start: u64, packet: &AudioPacket) {
    let lo=start.max(packet.start);
    let hi=(start+output.len() as u64).min(packet.start+packet.samples.len() as u64);
    if hi<=lo {return;}
    for i in lo..hi { output[(i-start) as usize]+=packet.samples[(i-packet.start) as usize]; }
}

pub fn pcm16(mix: &[f32]) -> Vec<u8> {
    mix.iter().flat_map(|sample| ((sample.clamp(-1.0,1.0)*i16::MAX as f32) as i16).to_le_bytes()).collect()
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn timestamps_do_not_depend_on_encoder_throughput() {
        assert_eq!(frame_time(10),sample_time(48000)); assert_eq!(frame_time(35),35_000_000);
    }
    #[test] fn mixing_aligns_endpoints_and_clips_late_packets() {
        let mut out=vec![0.0;4]; mix_packet(&mut out,10,&AudioPacket{start:8,samples:vec![0.5;4]});
        mix_packet(&mut out,10,&AudioPacket{start:11,samples:vec![0.5;4]});
        assert_eq!(out,vec![0.5,1.0,0.5,0.5]);
    }
    #[test] fn silent_devices_and_clipping_are_safe() {
        assert_eq!(pcm16(&[0.0,2.0,-2.0]),vec![0,0,255,127,1,128]);
    }
}
