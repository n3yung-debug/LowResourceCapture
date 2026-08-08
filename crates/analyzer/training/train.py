#!/usr/bin/env python3
"""Train the kill classifier and export it to ONNX.

Run offline on your own machine. This never ships in the installer — the
analyzer only loads the resulting .onnx.

    python train.py --data D:\\clipanalyzer-dataset --build 2026.08.06

Why a plain image classifier rather than an object detector: enemy cosmetics
don't render, so the visual vocabulary is closed — a given class in a given
armor tier looks identical every time, and each map's monster roster is fixed.
That's a small learning problem, and it trains on the K/M/D marks you already
make. No bounding boxes needed.

The split is BY SOURCE VIDEO, not by frame. Frames from one death are nearly
identical, so a random split would put near-duplicates on both sides and
report an accuracy that means nothing. Held-out numbers here are only
meaningful because whole recordings are held out.
"""
import argparse
import collections
import datetime
import json
import pathlib
import random
import sys

try:
    import torch
    import torch.nn as nn
    from torch.utils.data import DataLoader, Dataset
    import torchvision.transforms as T
    from torchvision.models import mobilenet_v3_small, MobileNet_V3_Small_Weights
    from PIL import Image
except ImportError:
    sys.exit(
        "Missing dependencies. Install with:\n"
        "  pip install torch torchvision pillow --index-url "
        "https://download.pytorch.org/whl/cu124"
    )


class Frames(Dataset):
    def __init__(self, examples, root, classes, train):
        self.examples = examples
        self.root = pathlib.Path(root)
        self.class_to_idx = {c: i for i, c in enumerate(classes)}
        # Light augmentation only. No horizontal flip: the HUD is not mirror
        # symmetric, and teaching the model that a flipped HUD is normal
        # throws away a real, stable cue.
        if train:
            self.tf = T.Compose([
                T.ColorJitter(brightness=0.25, contrast=0.25, saturation=0.15),
                T.ToTensor(),
                T.Normalize([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]),
                T.RandomErasing(p=0.25, scale=(0.02, 0.12)),
            ])
        else:
            self.tf = T.Compose([
                T.ToTensor(),
                T.Normalize([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]),
            ])

    def __len__(self):
        return len(self.examples)

    def __getitem__(self, i):
        e = self.examples[i]
        img = Image.open(self.root / e["file"]).convert("RGB")
        return self.tf(img), self.class_to_idx[e["class"]]


def split_by_source(examples, holdout_frac=0.25, seed=0):
    """Hold out whole recordings, never individual frames."""
    sources = sorted({e["source"] for e in examples})
    if len(sources) < 2:
        print(
            f"WARNING: only {len(sources)} source recording(s). A held-out score\n"
            "         needs at least two, and really wants several. Training will\n"
            "         run but the accuracy number will not mean anything yet.",
            file=sys.stderr,
        )
        return examples, []
    rng = random.Random(seed)
    rng.shuffle(sources)
    n_hold = max(1, int(len(sources) * holdout_frac))
    held = set(sources[:n_hold])
    return ([e for e in examples if e["source"] not in held],
            [e for e in examples if e["source"] in held])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True, help="dataset root (contains manifest.json)")
    ap.add_argument("--out", default=None, help="output .onnx (default: <data>/model.onnx)")
    ap.add_argument("--build", default="", help="current game build, recorded on the card")
    ap.add_argument("--epochs", type=int, default=12)
    ap.add_argument("--batch", type=int, default=32)
    ap.add_argument("--lr", type=float, default=3e-4)
    args = ap.parse_args()

    root = pathlib.Path(args.data)
    manifest = json.loads((root / "manifest.json").read_text())
    examples = manifest["examples"]
    if not examples:
        sys.exit("manifest.json has no examples — export a reviewed recording first.")

    counts = collections.Counter(e["class"] for e in examples)
    classes = sorted(counts)
    print(f"{len(examples)} frames across {len(classes)} classes:")
    for c in classes:
        print(f"  {c:10} {counts[c]}")

    thin = [c for c in classes if counts[c] < 50]
    if thin:
        print(f"\nWARNING: thin classes {thin} (<50 frames). Expect these to be\n"
              "         unreliable no matter what the overall accuracy says.\n",
              file=sys.stderr)

    train_ex, held_ex = split_by_source(examples)
    print(f"\ntrain {len(train_ex)} frames / held-out {len(held_ex)} frames")

    dev = "cuda" if torch.cuda.is_available() else "cpu"
    print(f"device: {dev}")

    model = mobilenet_v3_small(weights=MobileNet_V3_Small_Weights.DEFAULT)
    model.classifier[3] = nn.Linear(model.classifier[3].in_features, len(classes))
    model = model.to(dev)

    # Weight by inverse frequency so a rare class isn't simply ignored — with
    # far more "none" frames than kills, an unweighted model scores well by
    # never predicting a kill at all.
    weights = torch.tensor(
        [len(examples) / (len(classes) * counts[c]) for c in classes],
        dtype=torch.float32, device=dev,
    )
    loss_fn = nn.CrossEntropyLoss(weight=weights)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr)

    train_dl = DataLoader(Frames(train_ex, root, classes, True),
                          batch_size=args.batch, shuffle=True, num_workers=0)
    held_dl = (DataLoader(Frames(held_ex, root, classes, False),
                          batch_size=args.batch, num_workers=0) if held_ex else None)

    for epoch in range(args.epochs):
        model.train()
        total = correct = 0
        running = 0.0
        for x, y in train_dl:
            x, y = x.to(dev), y.to(dev)
            opt.zero_grad()
            out = model(x)
            loss = loss_fn(out, y)
            loss.backward()
            opt.step()
            running += loss.item() * len(y)
            correct += (out.argmax(1) == y).sum().item()
            total += len(y)
        print(f"epoch {epoch+1:2}/{args.epochs}  loss {running/max(total,1):.4f}  "
              f"train acc {correct/max(total,1):.3f}")

    accuracy, recall = None, {}
    if held_dl:
        model.eval()
        per_class = collections.defaultdict(lambda: [0, 0])  # [correct, total]
        correct = total = 0
        with torch.no_grad():
            for x, y in held_dl:
                x, y = x.to(dev), y.to(dev)
                pred = model(x).argmax(1)
                for t, p in zip(y.tolist(), pred.tolist()):
                    per_class[classes[t]][1] += 1
                    if t == p:
                        per_class[classes[t]][0] += 1
                correct += (pred == y).sum().item()
                total += len(y)
        accuracy = correct / max(total, 1)
        print(f"\nheld-out accuracy {accuracy:.3f}")
        for c in classes:
            got, n = per_class[c]
            if n:
                recall[c] = got / n
                print(f"  {c:10} recall {got/n:.3f}  ({got}/{n})")

    out_path = pathlib.Path(args.out) if args.out else root / "model.onnx"
    dummy = torch.randn(1, 3, manifest["frame_height"], manifest["frame_width"], device=dev)
    model.eval()
    torch.onnx.export(
        model, dummy, str(out_path),
        input_names=["input"], output_names=["logits"],
        dynamic_axes={"input": {0: "batch"}, "logits": {0: "batch"}},
        opset_version=17,
    )

    builds = sorted({e["game_build"] for e in examples if e.get("game_build")})
    card = {
        "trained_at": datetime.date.today().isoformat(),
        "game_builds": builds or ([args.build] if args.build else []),
        "classes": classes,
        "examples_per_class": dict(counts),
        "input_width": manifest["frame_width"],
        "input_height": manifest["frame_height"],
        "holdout_accuracy": accuracy,
        "holdout_recall": recall,
        "notes": "mobilenet_v3_small, split by source recording",
    }
    card_path = out_path.with_suffix(".card.json")
    card_path.write_text(json.dumps(card, indent=2))
    print(f"\nwrote {out_path}\nwrote {card_path}")


if __name__ == "__main__":
    main()
