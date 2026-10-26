# MIPRES: usar ID de entrega en la radicación

Branch: current working branch (no branch switch).
Delivery strategy: ask-on-risk.
Test runner: `cargo test`.

## Specs

S1. El usuario dijo: "la validacion es correcta, el id de entrega es el que se exige al momento de radicar en Mutual, corrigelo. todo el comportamiento que tienes hasta ahora es correcto, solamente que al obtener la respuesta de mipres tomaras ese id de entrega para usarlo en la radicacion, cambiando el que usas actualmente."

S2. La respuesta del endpoint MIPRES `GET {base}/api/EntregaXPrescripcion/{nit}/{token}/{prescripcion}` trae por fila los campos `ID`, `IDEntrega`, `NoPrescripcion`, `EstEntrega`. Hoy el decodificador extrae solo `ID` y `EstEntrega` (`src/phases/convenio_preparar/mipres_client.rs:434-437`); `IDEntrega` se ignora.

S3. La validación contra MIPRES NO cambia: el match de cada línea por No. Formula con `same_delivery_id` queda igual, y solo `EstEntrega = ENTREGADO` (estado 2) pasa. Lo único que cambia es qué id se guarda y se envía.

S4. Al preparar (`check_line`, `src/phases/convenio_preparar/mipres.rs:213-214`), al pasar una línea ENTREGADA se debe guardar el **ID de entrega** (`IDEntrega` de la respuesta) en lugar del campo `ID`. `Invoice.id_mipres` sigue siendo la lista de ids separados por `", "` (una factura puede tener varias líneas NO PBS; `check_invoice`, `mipres.rs:147-174`). No cambiar el formato del campo persistido.

S5. En la fase RIPS (paso 3 de radicación), hoy `id_mipres()` envía `normalize_id_mipres(l.formula_number)` (`src/phases/rips/builder.rs:343-350`). Debe cambiar a enviar el **id de entrega almacenado en la factura** (`Invoice.id_mipres`), manteniendo la regla de que solo aplica cuando la línea tiene valor en la columna `mipres`.

S6. Mapeo línea → id: los ids guardados en `Invoice.id_mipres` siguen el orden de las líneas NO PBS procesadas por `check_invoice` (orden del cargue). El builder del RIPS debe asociar a cada línea NO PBS su id de entrega correspondiente por ese orden. Si la cuenta de ids no coincide con el número de líneas NO PBS de la factura, es un error de datos: detener con mensaje claro, no adivinar.

S7. La normalización de ceros a la izquierda (`normalize_delivery_id` / `same_delivery_id`) sigue aplicando solo a la validación (match contra No. Formula). El id de entrega se persiste y se envía tal cual viene del servicio.

## Tasks

- T1 (delegated, writer): S2-S7. Cambiar modelo `Delivery`, decodificador, `check_line`, builder RIPS y tests. DONE 2026-10-26: `Delivery.delivery_id` (IDEntrega) extraído y guardado; builder RIPS recibe `RipsInput.id_mipres` (contrato interno extendido: input.rs, selection.rs, plan.rs, tests/rips_phase.rs) y mapea por orden de líneas NO PBS con error claro ante conteo disparejo. Commit pendiente (el usuario decide commit/push).

## Log

L1 (2026-10-26): "la validacion es correcta, el id de entrega es el que se exige al momento de radircar en Mutual, corrigelo. todo el comportamiento que tienes hasta ahora es correcto, solamente que al obtener la respuesta de mipres tomaras ese id de entrega para usarlo en la radicacion, cambiando el que usas actualmente. dame los pasos que usarias teniendo en cuenta este cambio" — seguido de "perfecto, ya entendi, vamos, genera la tarea para poder lanzar el agente que la trabajara".

L2 (2026-10-26), evidencia T1: RED observado (tests actualizados con `IDEntrega` distintos del `ID` fallaron/compilaron en rojo contra el código viejo), luego GREEN. `cargo test convenio_preparar`: 87 passed / 0 failed. `cargo test --lib rips::`: 29 passed / 0 failed. Suite `cargo test --lib`: 773 passed / 94 failed — baseline con `git stash` (sin el cambio): 771 passed / 94 failed; los 94 son fallos ambientales preexistentes (Postgres de prueba 127.0.0.1:55432 y Redis/S3 no levantados, `src/db/test_support.rs:52`), idénticos en ambos lados. Sin commit (pendiente decisión del usuario).
