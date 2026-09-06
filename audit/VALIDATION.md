# VALIDATION — Smoke en Void Real (ejecuta el HUMANO)

**Contexto:** CI (ubuntu) no puede correr lo interactivo ni lo que exige Void.
Esta máquina SÍ es Void real con `vary` instalado: ejecutar aquí, PC enchufado.
**Toda esta fase es de LECTURA + comandos inocuos salvo donde se indica.**

---

## 0. Seguridad previa (OBLIGATORIO, 2 min)

```sh
sudo cp -a /etc/xbps.d /root/xbps.d.pre-audit
cp -a ~/.cache/vary /tmp/vary-cache-backup 2>/dev/null
cp -a ~/.config/vary /tmp/vary-config-backup 2>/dev/null
cp -a ~/.local/share/vary /tmp/vary-data-backup 2>/dev/null
vary -V
git -C ~/.../Vary status --short   # el árbol debe estar limpio en f08b761
```

Rollback si algo se tuerce: `sudo rm -rf /etc/xbps.d && sudo cp -a /root/xbps.d.pre-audit /etc/xbps.d`.

---

## 1. Tests ignorados (los 4 que CI no puede correr)

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -- --ignored
```

Esperado: los 4 `xbps::tests::integracion_*` en verde (usan repos reales).
Si alguno falla, anotar salida exacta: es regresión contra xbps real.

---

## 2. `-Syu` con bulk cache + niveles de log

```sh
VARY_DEBUG=1 vary -Syu 2>&1 | tail -20
vary -v -Ss foo 2>&1 | head -5   # H-028: con -v debe haber líneas debug
```

Esperado: sync oficial + refresh VURs sin errores; `-v` muestra más que sin él.

---

## 3. Install VUR con review gate + diff gate (A3)

Elegir un paquete VUR PEQUEÑO ya instalado con versión vieja, o instalar uno
nuevo pequeño:

```sh
vary -S <pkg-pequeño-vur>        # debe mostrar template (review) y pedir sí
echo $?                          # 0
vary -S <pkg-pequeño-vur> < /dev/null; echo $?   # H-019: 1 + hint --noconfirm en stderr
```

Si hay upgrade con cambio de template pendiente: `vary -Syu` debe mostrar el
diff unificado y pedir sí por paquete (A3). Con `</dev/null` debe abortar
mencionando `--yes`.

---

## 4. Ctrl+C a mitad de build (H-016 + H-034)

```sh
vary -S <pkg-que-compile> &
sleep 20; kill -INT %1          # o Ctrl+C si es interactivo
sleep 2
ps aux | grep -E "xbps-src|make|ninja|cc1" | grep -v grep   # ESPERADO: vacío (sin huérfanos)
ls /tmp/vary-* 2>/dev/null      # ESPERADO: vacío (sin temporales)
# cursor visible y terminal sana (H-034): escribir algo y verificar eco
```

---

## 5. Doble instancia (H-027)

```sh
vary -S <pkg-grande> &          # mantiene el lock minutos
sleep 5
vary -Ss foo; echo "search: $?" # ESPERADO: funciona (0), sin lock
vary -V; echo "version: $?"     # ESPERADO: funciona (0), sin lock
vary -S otro-pkg; echo "2nd install: $?"  # ESPERADO: error "otra instancia" SIN la palabra "borra"
wait
```

---

## 6. Upgrade desde installed.json viejo (migración H-005)

```sh
cp ~/.cache/vary/installed.json /tmp/installed.json.new
echo '{"hello-vur":{"version":"1.0","vur":"x","install_date":1700000000,"install_type":"Source"}}' > ~/.cache/vary/installed.json
vary -Syu 2>&1 | tail -3         # no debe perder la entrada ni abortar
python3 -c "import json; print(json.load(open('$HOME/.cache/vary/installed.json'))['hello-vur'])"
# ESPERADO: build_date == 1700000000000 (backfill), schema_version 2
cp /tmp/installed.json.new ~/.cache/vary/installed.json
```

---

## 7. Flujo no-TTY + códigos de salida (H-019, H-035, H-032/33)

```sh
vary --opcion-inexistente-xyz >/dev/null 2>&1; echo "badflag: $?"   # ESPERADO: 2
vary -S foo --asdeps >/dev/null 2>&1; echo "asdeps: $?"             # ESPERADO: 1 (rechazo, no silencio)
vary -S foo --config /tmp/x >/dev/null 2>&1; echo "config: $?"      # ESPERADO: 1 + "no está soportada"
```

---

## 8. Cierre de fase

Anotar en `audit/FIX_LOG.md` (apéndice FASE 5): fecha, máquina (`xbps-query -R -p architecture`), resultados 1-7 (OK/FALLO + salida). Solo con 1-7 en verde se da por cerrada FASE 5.
