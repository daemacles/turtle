-- Sparse Conway Game of Life example running on an unbounded integer plane.
-- Uses single-column tiles and tick pacing with zero entities so arrow keys pan the viewport and
-- pressing E toggles the cell beneath the center reticle.

--- World module exported to the host engine.
local world = {}

--- Display name shown in the top status bar.
world.title = "Game of Life"

--- Single-column tile mode fits dense cellular patterns inside the viewport.
world.cell_width = 1

--- Fixed-interval clock driving continuous generations unless paused.
world.pacing = { mode = "tick", tick_ms = 140 }

--- Visual legend distinguishing live cells from faint background grid dots.
world.palette = {
    ["#"] = { "█", "BrightGreen", "Black" },
    ["."] = { "·", "BrightBlack", "Black" },
}

--- Sparse map of living cells keyed by coordinate string.
---@type table<string, { x: integer, y: integer }>
local alive = {}

--- Live cell count tracked for HUD reporting.
local live_count = 0

--- Encodes signed coordinates into a canonical sparse table key.
---@param x integer Horizontal cell coordinate.
---@param y integer Vertical cell coordinate.
---@return string key Stable string key representing the cell coordinate.
local function cell_key(x, y)
    return tostring(x) .. "," .. tostring(y)
end

--- Marks or clears one cell while maintaining the live count.
---@param x integer Horizontal cell coordinate.
---@param y integer Vertical cell coordinate.
---@param state boolean True to mark alive or false to clear.
local function set_cell(x, y, state)
    local k = cell_key(x, y)
    if state then
        if not alive[k] then
            alive[k] = { x = x, y = y }
            live_count = live_count + 1
        end
    else
        if alive[k] then
            alive[k] = nil
            live_count = live_count - 1
        end
    end
end

--- Stamps an ASCII pattern centered near the provided origin coordinates.
---@param ox integer Left anchor X coordinate.
---@param oy integer Top anchor Y coordinate.
---@param rows string[] Pattern lines where hash marks indicate live cells.
local function stamp_pattern(ox, oy, rows)
    for dy, row in ipairs(rows) do
        for dx = 1, #row do
            if row:sub(dx, dx) == "#" then
                set_cell(ox + dx - 1, oy + dy - 1, true)
            end
        end
    end
end

--- Seeds a glider squadron and small pseudorandom soup around the origin.
---@param seed integer Deterministic random seed passed by the host engine.
---@return table[] entities Empty entity list since cellular automata use terrain only.
---@return table hud Initial live cell count and control hint.
function world.init(seed)
    alive = {}
    live_count = 0

    -- Place an R-pentomino and two gliders around origin for immediate long-lived activity.
    stamp_pattern(-2, -1, {
        ".##",
        "##.",
        ".#.",
    })
    stamp_pattern(-18, -5, {
        ".#.",
        "..#",
        "###",
    })
    stamp_pattern(12, 3, {
        ".#.",
        "#..",
        "###",
    })

    -- Sprinkle a small seeded cluster based on seed so pressing R yields distinct colonies.
    local rng_bias = (seed % 5) + 4
    for dy = -3, 3 do
        for dx = 18, 24 do
            if math.random(1, 10) <= rng_bias then
                set_cell(dx, dy, true)
            end
        end
    end

    return {}, {
        status = "CELLS " .. tostring(live_count),
        msg = "WASD/Arrows pan viewport; E toggles center cell; P pauses.",
    }
end

--- Advances one Conway generation or toggles the center cell on act requests.
---@param _t integer Current one-based simulation step counter.
---@param requests table[] Optional cursor toggle intents emitted when pressing act.
---@param _entities table<string, table> Unused entity map for playerless automata.
---@return table[] patches Always empty since automata state lives inside the cell set.
---@return table hud Updated cell count and optional toggle confirmation message.
function world.step(_t, requests, _entities)
    local msg = nil
    for _, req in ipairs(requests) do
        if req.action == "act" then
            local k = cell_key(req.x, req.y)
            local next_state = not alive[k]
            set_cell(req.x, req.y, next_state)
            if next_state then
                msg = "Activated cell (" .. tostring(req.x) .. "," .. tostring(req.y) .. ")"
            else
                msg = "Cleared cell (" .. tostring(req.x) .. "," .. tostring(req.y) .. ")"
            end
            return {}, { status = "CELLS " .. tostring(live_count), msg = msg }
        end
    end

    local counts = {}
    local coords_x = {}
    local coords_y = {}

    for _, cell in pairs(alive) do
        for oy = -1, 1 do
            for ox = -1, 1 do
                if not (ox == 0 and oy == 0) then
                    local nx, ny = cell.x + ox, cell.y + oy
                    local nk = cell_key(nx, ny)
                    counts[nk] = (counts[nk] or 0) + 1
                    coords_x[nk] = nx
                    coords_y[nk] = ny
                end
            end
        end
    end

    local next_alive = {}
    local next_count = 0
    for nk, n_count in pairs(counts) do
        if n_count == 3 or (n_count == 2 and alive[nk]) then
            next_alive[nk] = { x = coords_x[nk], y = coords_y[nk] }
            next_count = next_count + 1
        end
    end

    alive = next_alive
    live_count = next_count

    return {}, { status = "CELLS " .. tostring(live_count), msg = msg }
end

--- Returns the visible tile matrix covering the requested world bounding box.
---@param start_pos table Top-left corner containing integer `x` and `y` coordinates.
---@param end_pos table Bottom-right corner containing integer `x` and `y` coordinates.
---@param _t integer Unused simulation step counter.
---@return string[][] rows Two-dimensional row-major array of palette keys.
function world.view(start_pos, end_pos, _t)
    local rows = {}
    local ry = 1
    for y = start_pos.y, end_pos.y do
        local row = {}
        local rx = 1
        for x = start_pos.x, end_pos.x do
            row[rx] = alive[cell_key(x, y)] and "#" or "."
            rx = rx + 1
        end
        rows[ry] = row
        ry = ry + 1
    end
    return rows
end

return world
