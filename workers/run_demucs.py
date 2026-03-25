#!/usr/bin/env python3
"""Run Demucs stem separation, saving output as WAV via soundfile.

Avoids the torchcodec dependency issue in torchaudio 2.11+.
Usage: python3 run_demucs.py <input_file> <output_dir> [model_name]
"""
import sys
import os

def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <input_file> <output_dir> [model_name]", file=sys.stderr)
        sys.exit(1)

    input_path = sys.argv[1]
    output_dir = sys.argv[2]
    model_name = sys.argv[3] if len(sys.argv) > 3 else "htdemucs"

    import torch
    import torchaudio
    import soundfile as sf
    import numpy as np

    # Load model
    print(f"Loading model {model_name}...", flush=True)
    from demucs.pretrained import get_model
    model = get_model(model_name)
    model.eval()

    device = "cuda" if torch.cuda.is_available() else "cpu"
    print(f"Using device: {device}", flush=True)
    model.to(device)

    # Load audio
    print(f"Loading audio: {input_path}", flush=True)
    # Use soundfile backend for loading too
    wav_data, sr = sf.read(input_path, dtype='float32', always_2d=True)
    # Convert to torch tensor (channels, samples)
    wav = torch.from_numpy(wav_data.T).float()
    if wav.dim() == 1:
        wav = wav.unsqueeze(0)
    # Ensure stereo
    if wav.shape[0] == 1:
        wav = wav.repeat(2, 1)

    # Resample to model's sample rate if needed
    model_sr = model.samplerate
    if sr != model_sr:
        print(f"Resampling {sr}Hz -> {model_sr}Hz...", flush=True)
        wav = torchaudio.functional.resample(wav, sr, model_sr)

    # Add batch dimension
    wav = wav.unsqueeze(0).to(device)

    # Separate
    print("Separating stems...", flush=True)
    with torch.no_grad():
        from demucs.apply import apply_model
        sources = apply_model(model, wav, progress=True)

    # sources shape: (batch, num_sources, channels, samples)
    sources = sources[0]  # remove batch dim

    # Save stems
    stem_names = model.sources  # e.g. ['drums', 'bass', 'other', 'vocals']
    print(f"Stems: {stem_names}", flush=True)

    os.makedirs(output_dir, exist_ok=True)

    for i, name in enumerate(stem_names):
        stem = sources[i].cpu().numpy().T  # (samples, channels)
        stem_path = os.path.join(output_dir, f"{name}.wav")
        sf.write(stem_path, stem, model_sr)
        print(f"Saved: {stem_path}", flush=True)

    print("Done.", flush=True)

if __name__ == "__main__":
    main()
