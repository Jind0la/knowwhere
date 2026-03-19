import { useState, useEffect } from 'react';
import { getGovernancePolicy, updateGovernancePolicy } from '../api/knowwhere';
import type { GovernancePolicy, Sensitivity } from '../types';

const SENSITIVITY_OPTIONS: Sensitivity[] = ['normal', 'low', 'high', 'restricted'];

export function GovernancePanel() {
  const [policy, setPolicy] = useState<GovernancePolicy | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Editable form state
  const [minConf, setMinConf] = useState(0.5);
  const [maxAge, setMaxAge] = useState<string>('');
  const [blocked, setBlocked] = useState<Sensitivity[]>([]);
  const [supersession, setSupersession] = useState(true);
  const [conflictCheck, setConflictCheck] = useState(true);
  const [recencyBoost, setRecencyBoost] = useState(true);
  const [penaltyDays, setPenaltyDays] = useState(90);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const data = await getGovernancePolicy();
      setPolicy(data);
      setMinConf(data.min_confidence);
      setMaxAge(data.max_age_days?.toString() ?? '');
      setBlocked(data.blocked_sensitivities);
      setSupersession(data.supersession_enabled);
      setConflictCheck(data.conflict_check_enabled);
      setRecencyBoost(data.recency_boost_enabled);
      setPenaltyDays(data.recency_penalty_after_days);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    load();
  }, []);

  async function handleSave(preset?: string) {
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      const res = await updateGovernancePolicy({
        ...(preset ? { preset } : {}),
        min_confidence: minConf,
        max_age_days: maxAge ? Number(maxAge) : undefined,
        blocked_sensitivities: blocked,
        supersession_enabled: supersession,
        conflict_check_enabled: conflictCheck,
        recency_boost_enabled: recencyBoost,
        recency_penalty_after_days: penaltyDays,
      });
      setPolicy(res.policy as GovernancePolicy);
      setSaved(true);
      setTimeout(() => setSaved(false), 3000);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  function toggleBlocked(s: Sensitivity) {
    setBlocked((prev) =>
      prev.includes(s) ? prev.filter((x) => x !== s) : [...prev, s]
    );
  }

  if (loading && !policy) {
    return <p className="text-sm text-gray-400 p-3">Loading governance policy…</p>;
  }

  return (
    <div className="p-4 space-y-4">
      <h2 className="text-sm font-semibold text-gray-700 uppercase tracking-wide">
        Governance Policy
      </h2>

      {error && <p className="text-xs text-red-500">{error}</p>}

      {saved && (
        <p className="text-xs text-green-600 bg-green-50 px-3 py-2 rounded">
          Policy saved successfully.
        </p>
      )}

      {/* Presets */}
      <div className="flex gap-2">
        {(['default', 'strict', 'lenient'] as const).map((p) => (
          <button
            key={p}
            className="text-xs px-3 py-1.5 rounded border border-gray-300 hover:border-indigo-400 hover:text-indigo-600 transition capitalize"
            onClick={() => handleSave(p)}
            disabled={saving}
          >
            {p}
          </button>
        ))}
      </div>

      {/* Fields */}
      <div className="space-y-3">
        <div>
          <label className="text-xs text-gray-600 block mb-1">
            Min Confidence: {minConf.toFixed(2)}
          </label>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={minConf}
            onChange={(e) => setMinConf(Number(e.target.value))}
            className="w-full"
          />
        </div>

        <div>
          <label className="text-xs text-gray-600 block mb-1">
            Max Age (days, empty = no limit)
          </label>
          <input
            type="number"
            className="w-full border border-gray-300 rounded px-2 py-1 text-sm"
            value={maxAge}
            placeholder="No limit"
            onChange={(e) => setMaxAge(e.target.value)}
          />
        </div>

        <div>
          <label className="text-xs text-gray-600 block mb-1">
            Blocked Sensitivities
          </label>
          <div className="flex flex-wrap gap-2">
            {SENSITIVITY_OPTIONS.map((s) => (
              <label key={s} className="text-xs flex items-center gap-1 cursor-pointer">
                <input
                  type="checkbox"
                  checked={blocked.includes(s)}
                  onChange={() => toggleBlocked(s)}
                />
                {s}
              </label>
            ))}
          </div>
        </div>

        <div className="space-y-1">
          {[
            ['supersession_enabled', supersession, setSupersession, 'Enable supersession check'],
            ['conflict_check_enabled', conflictCheck, setConflictCheck, 'Enable conflict check'],
            ['recency_boost_enabled', recencyBoost, setRecencyBoost, 'Enable recency boost'],
          ].map(([key, val, setVal, label]) => (
            <label key={key as string} className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={val as boolean}
                onChange={(e) => (setVal as (v: boolean) => void)(e.target.checked)}
              />
              <span className="text-sm text-gray-700">{label}</span>
            </label>
          ))}
        </div>

        <div>
          <label className="text-xs text-gray-600 block mb-1">
            Recency Penalty After (days): {penaltyDays}
          </label>
          <input
            type="range"
            min={7}
            max={365}
            step={7}
            value={penaltyDays}
            onChange={(e) => setPenaltyDays(Number(e.target.value))}
            className="w-full"
          />
        </div>
      </div>

      <button
        className="w-full bg-indigo-600 text-white text-sm py-2 rounded hover:bg-indigo-700 disabled:opacity-50"
        onClick={() => handleSave()}
        disabled={saving}
      >
        {saving ? 'Saving…' : 'Apply Custom Policy'}
      </button>
    </div>
  );
}
