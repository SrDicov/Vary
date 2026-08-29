import re
with open("src/vur_client.rs", "r") as f:
    text = f.read()
text = text.replace("Result<&[VurInfo]>", "Result<Vec<VurInfo>>")
text = text.replace("cache.store(&key, valid);\n        Ok(cache.load_valid(&key, None).unwrap())", "cache.store(&key, valid.clone());\n        Ok(valid)")
with open("src/vur_client.rs", "w") as f:
    f.write(text)
