"""Generate JV-02 synthetic seed corpus (20 rows, CC0-1.0).

Run once to regenerate `synthetic_seed.jsonl` + `synthetic_seed_manifest.json`.
The generator is committed so the seed is reproducible; the fixture files are
the canonical artefacts the validator/scorer consume.

No network. Pure stdlib.
"""
from __future__ import annotations

import hashlib
import json
import unicodedata
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent

LICENSE = "CC0-1.0"
LICENSE_URL = "https://creativecommons.org/publicdomain/zero/1.0/"
ATTRIBUTION = "tui-translator JV-02 synthetic seed (CC0-1.0, no PII)"
SOURCE = "synthetic"

# (numeric_id, category, ja, [vi_refs...])
# All sentences are hand-written for this project. They contain no real names,
# no real organisations, no meeting content, no PII, no secrets.
ROWS: list[tuple[int, str, str, list[str]]] = [
    # short ───────────────────────────────────────────────────────────────────
    (1, "short", "おはようございます。",
        ["Chào buổi sáng."]),
    (2, "short", "了解しました。",
        ["Tôi đã hiểu rồi.", "Đã rõ."]),
    (3, "short", "もう一度お願いします。",
        ["Xin vui lòng nói lại một lần nữa."]),
    # medium ──────────────────────────────────────────────────────────────────
    (4, "medium", "今日の会議の議題を共有してもよろしいでしょうか。",
        ["Tôi có thể chia sẻ chương trình nghị sự của cuộc họp hôm nay được không?"]),
    (5, "medium", "ネットワークが不安定なので、音声が途切れるかもしれません。",
        ["Vì mạng không ổn định nên âm thanh có thể bị ngắt quãng."]),
    (6, "medium", "資料は後ほどメールでお送りします。",
        ["Tài liệu tôi sẽ gửi qua email sau."]),
    # long ────────────────────────────────────────────────────────────────────
    (7, "long",
        "先週議論した設計案について、いくつかの懸念点が残っていますので、"
        "本日の打ち合わせで優先順位を決めて、来週までに方針を確定したいと考えています。",
        ["Về phương án thiết kế mà chúng ta đã thảo luận tuần trước, vẫn còn một số "
         "điểm lo ngại, vì vậy trong buổi họp hôm nay tôi muốn quyết định thứ tự "
         "ưu tiên và chốt phương án vào tuần sau."]),
    (8, "long",
        "もし時間が許せば、現在のレイテンシ計測の結果と、ローカルモデルへの"
        "切り替えによる影響についても、続けて議論したいと思います。",
        ["Nếu thời gian cho phép, tôi cũng muốn tiếp tục thảo luận về kết quả đo "
         "độ trễ hiện tại và ảnh hưởng của việc chuyển sang mô hình cục bộ."]),
    # honorific ───────────────────────────────────────────────────────────────
    (9, "honorific", "ご確認いただけますでしょうか。",
        ["Anh/chị có thể vui lòng kiểm tra giúp tôi được không ạ?",
         "Xin anh/chị vui lòng xác nhận."]),
    (10, "honorific", "お忙しいところ恐れ入りますが、ご返信をお待ちしております。",
        ["Tôi rất ngại vì đã làm phiền lúc anh/chị bận, nhưng tôi xin chờ phản hồi của anh/chị."]),
    (11, "honorific", "本日はお時間をいただき、誠にありがとうございました。",
        ["Hôm nay tôi xin chân thành cảm ơn anh/chị đã dành thời gian."]),
    # disfluent ───────────────────────────────────────────────────────────────
    (12, "disfluent", "えーっと、その、つまり、結論としては反対です。",
        ["Ờm, tức là, nói tóm lại thì tôi phản đối."]),
    (13, "disfluent", "あの、ちょっと、音声が、聞こえにくいんですけど…",
        ["Ờ, một chút, âm thanh thì hơi khó nghe…"]),
    # technical ──────────────────────────────────────────────────────────────
    (14, "technical", "サンプリングレートを16キロヘルツに設定してください。",
        ["Vui lòng đặt tần số lấy mẫu thành 16 kHz."]),
    (15, "technical", "WASAPIループバックでステレオの音声を取得しています。",
        ["Chúng tôi đang lấy âm thanh stereo bằng WASAPI loopback."]),
    (16, "technical", "p95レイテンシが3秒を超えた場合は、ローカル経路にフォールバックします。",
        ["Khi độ trễ p95 vượt quá 3 giây, hệ thống sẽ chuyển sang đường cục bộ dự phòng."]),
    # named-entity (project-internal synthetic names only) ────────────────────
    (17, "named-entity",
        "次のリリースでは、ローカル翻訳エンジン「ニジ」を既定にする予定です。",
        ["Trong bản phát hành tiếp theo, chúng tôi dự định đặt động cơ dịch cục bộ "
         "“Niji” làm mặc định."]),
    (18, "named-entity",
        "サンプル会社「コーラル商事」のロゴが共有画面に映っています。",
        ["Logo của công ty mẫu “Coral Shoji” đang hiển thị trên màn hình chia sẻ."]),
    (19, "named-entity",
        "テスト用ユーザー「ユーザーA」と「ユーザーB」のアカウントを作成しました。",
        ["Tôi đã tạo tài khoản người dùng kiểm thử “người dùng A” và “người dùng B”."]),
    (20, "short", "失礼します。",
        ["Xin phép."]),
]


def canonical_id(n: int) -> str:
    return f"jv-syn-{n:06d}"


def build_rows():
    seen_ids: set[str] = set()
    out = []
    for n, category, ja, vi_refs in ROWS:
        ja_nfc = unicodedata.normalize("NFC", ja)
        vi_refs_nfc = [unicodedata.normalize("NFC", v) for v in vi_refs]
        rid = canonical_id(n)
        if rid in seen_ids:
            raise SystemExit(f"duplicate id: {rid}")
        seen_ids.add(rid)
        if not ja_nfc.strip():
            raise SystemExit(f"empty ja at {rid}")
        if not vi_refs_nfc or any(not v.strip() for v in vi_refs_nfc):
            raise SystemExit(f"empty vi_ref at {rid}")
        row = {
            "attribution": ATTRIBUTION,
            "category": category,
            "char_len_ja": len(ja_nfc),
            "id": rid,
            "ja": ja_nfc,
            "lang_src": "ja-JP",
            "lang_tgt": "vi-VN",
            "license": LICENSE,
            "license_url": LICENSE_URL,
            "source": SOURCE,
            "source_id": f"synthetic:v1:{rid}",
            "vi_refs": vi_refs_nfc,
            "added_at": "2026-05-21",
        }
        out.append(row)
    out.sort(key=lambda r: r["id"])
    return out


def serialise(rows) -> bytes:
    lines = []
    for row in rows:
        line = json.dumps(row, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        line = unicodedata.normalize("NFC", line)
        lines.append(line)
    text = "\n".join(lines) + "\n"
    return text.encode("utf-8")


def main() -> None:
    rows = build_rows()
    blob = serialise(rows)
    out_jsonl = HERE / "synthetic_seed.jsonl"
    out_jsonl.write_bytes(blob)

    sha = hashlib.sha256(blob).hexdigest()

    manifest = {
        "schema_version": "jv02-corpus-v1",
        "generated_at": datetime(2026, 5, 21, 0, 0, 0, tzinfo=timezone.utc)
            .strftime("%Y-%m-%dT%H:%M:%SZ"),
        "fixture_kind": "synthetic-seed",
        "row_count": len(rows),
        "corpus_sha256": sha,
        "rng_seed": 20260521,
        "round_count": 10,
        "round_order_seed": 20260521,
        "categories": sorted({r["category"] for r in rows}),
        "sources": [
            {
                "name": "synthetic",
                "version": "v1",
                "rows": len(rows),
                "sha256": sha,
                "license": LICENSE,
                "license_url": LICENSE_URL,
                "attribution": ATTRIBUTION,
            }
        ],
        "notes": (
            "This is the JV-02 synthetic seed fixture, not the full 300-row benchmark "
            "corpus.  See docs/evidence/ja-vi-benchmark-corpus-plan.md."
        ),
    }
    out_manifest = HERE / "synthetic_seed_manifest.json"
    out_manifest.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {out_jsonl} ({len(blob)} bytes, sha256={sha})")
    print(f"wrote {out_manifest}")


if __name__ == "__main__":
    main()
